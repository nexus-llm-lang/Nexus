use crate::ast::*;
use chumsky::Parser;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasmtime::*;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder};

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
    NativeFunction(String),
    Function(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(n) => write!(f, "{}", n),
            Value::Float(n) => write!(f, "{}", n),
            Value::Bool(b) => write!(f, "{}", b),
            Value::String(s) => write!(f, "{}", s),
            Value::Unit => write!(f, "()"),
            Value::Record(m) => {
                write!(f, "{{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k, v)?;
                }
                write!(f, "}}")
            }
            Value::Variant(name, args) => {
                write!(f, "{}", name)?;
                if !args.is_empty() {
                    write!(f, "(")?;
                    for (i, a) in args.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", a)?;
                    }
                    write!(f, ")")?;
                }
                Ok(())
            }
            Value::List(l) => {
                write!(f, "[")?;
                for (i, v) in l.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, "]")
            }
            Value::Array(a) => {
                write!(f, "[| ")?;
                for (i, v) in a.borrow().iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", v)?;
                }
                write!(f, " |]")
            }
            Value::Ref(_) => write!(f, "<ref>"),
            Value::NativeFunction(n) => write!(f, "<native fn {}>", n),
            Value::Function(n) => write!(f, "<fn {}>", n),
        }
    }
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
    pub native_functions: HashMap<String, Box<dyn Fn(&[Value]) -> Result<ExprResult, String>>>,
    pub external_functions: HashMap<String, ExternalFn>,
    pub wasm_store: RefCell<Store<WasiCtx>>,
    pub wasm_instances: Vec<Instance>,
    pub modules: HashMap<String, Interpreter>,
}

impl Interpreter {
    pub fn new(program: Program) -> Self {
        Self::new_with_stdlib(program, true)
    }

    fn new_with_stdlib(program: Program, load_stdlib: bool) -> Self {
        let mut functions = HashMap::new();
        let mut handlers = HashMap::new();
        let mut external_functions = HashMap::new();
        let mut modules = HashMap::new();
        let mut native_functions: HashMap<
            String,
            Box<dyn Fn(&[Value]) -> Result<ExprResult, String>>,
        > = HashMap::new();

        let engine = Engine::default();
        let wasi = WasiCtxBuilder::new().inherit_stdio().build();
        let mut store = Store::new(&engine, wasi);
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::add_to_linker(&mut linker, |s| s).expect("Failed to add WASI to linker");
        let mut wasm_instances = Vec::new();

        let mut all_definitions = Vec::new();
        if load_stdlib {
            if let Ok(stdlib_src) = std::fs::read_to_string("nxlib/stdlib/stdio.nx") {
                if let Ok(stdlib_program) = crate::parser::parser().parse(stdlib_src) {
                    all_definitions.extend(stdlib_program.definitions);
                }
            }
        }
        all_definitions.extend(program.definitions);

        for def in &all_definitions {
            match &def.node {
                TopLevel::Function(func) => {
                    functions.insert(func.name.clone(), func.clone());
                }
                TopLevel::Handler(handler) => {
                    handlers.insert(handler.port_name.clone(), handler.clone());
                }
                TopLevel::ExternalFn(ext) => {
                    external_functions.insert(ext.name.clone(), ext.clone());
                }
                TopLevel::Import(import) => {
                    if import.is_external {
                        let module = Module::from_file(&engine, &import.path)
                            .expect("Failed to load wasm module");
                        let instance = linker
                            .instantiate(&mut store, &module)
                            .expect("Failed to instantiate wasm module");
                        wasm_instances.push(instance);
                    } else {
                        let src =
                            std::fs::read_to_string(&import.path).expect("Failed to read module");
                        let p = crate::parser::parser()
                            .parse(src)
                            .expect("Failed to parse module");

                        let is_stdlib_module =
                            std::path::Path::new(&import.path).starts_with("nxlib/stdlib");
                        let sub_interp = Interpreter::new_with_stdlib(p, !is_stdlib_module);

                        if !import.items.is_empty() {
                            for item in &import.items {
                                if let Some(f) = sub_interp.functions.get(item) {
                                    functions.insert(item.clone(), f.clone());
                                }
                                if let Some(f) = sub_interp.external_functions.get(item) {
                                    external_functions.insert(item.clone(), f.clone());
                                }
                                // native functions? handlers?
                            }
                        } else {
                            let alias = import.alias.clone().unwrap_or_else(|| {
                                std::path::Path::new(&import.path)
                                    .file_stem()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or(&import.path)
                                    .to_string()
                            });
                            modules.insert(alias, sub_interp);
                        }
                    }
                }
                _ => {}
            }
        }

        // Register stdlib functions
        native_functions.insert(
            "i64_to_string".to_string(),
            Box::new(|args| {
                if args.len() != 1 {
                    return Some(Err("i64_to_string requires 1 arg".to_string())).unwrap();
                }
                if let Value::Int(i) = &args[0] {
                    Ok(ExprResult::Normal(Value::String(i.to_string())))
                } else {
                    Err("Expected i64".to_string())
                }
            }),
        );

        native_functions.insert(
            "float_to_string".to_string(),
            Box::new(|args| {
                if args.len() != 1 {
                    return Some(Err("float_to_string requires 1 arg".to_string())).unwrap();
                }
                if let Value::Float(f) = &args[0] {
                    Ok(ExprResult::Normal(Value::String(f.to_string())))
                } else {
                    Err("Expected float".to_string())
                }
            }),
        );

        native_functions.insert(
            "bool_to_string".to_string(),
            Box::new(|args| {
                if args.len() != 1 {
                    return Some(Err("bool_to_string requires 1 arg".to_string())).unwrap();
                }
                if let Value::Bool(b) = &args[0] {
                    Ok(ExprResult::Normal(Value::String(b.to_string())))
                } else {
                    Err("Expected bool".to_string())
                }
            }),
        );

        native_functions.insert(
            "drop_i64".to_string(),
            Box::new(|args| {
                if args.len() != 1 {
                    return Some(Err("drop_i64 requires 1 arg".to_string())).unwrap();
                }
                Ok(ExprResult::Normal(Value::Unit))
            }),
        );

        native_functions.insert(
            "drop_array".to_string(),
            Box::new(|args| {
                if args.len() != 1 {
                    return Some(Err("drop_array requires 1 arg".to_string())).unwrap();
                }
                Ok(ExprResult::Normal(Value::Unit))
            }),
        );

        native_functions.insert(
            "list_length".to_string(),
            Box::new(|args| {
                if args.len() != 1 {
                    return Some(Err("list_length requires 1 arg".to_string())).unwrap();
                }
                if let Value::List(xs) = &args[0] {
                    Ok(ExprResult::Normal(Value::Int(xs.len() as i64)))
                } else {
                    Err("Expected list".to_string())
                }
            }),
        );

        native_functions.insert(
            "array_length".to_string(),
            Box::new(|args| {
                if args.len() != 1 {
                    return Some(Err("array_length requires 1 arg".to_string())).unwrap();
                }
                if let Value::Array(arr) = &args[0] {
                    Ok(ExprResult::Normal(Value::Int(arr.borrow().len() as i64)))
                } else {
                    Err("Expected array".to_string())
                }
            }),
        );

        Interpreter {
            functions,
            handlers,
            native_functions,
            external_functions,
            wasm_store: RefCell::new(store),
            wasm_instances,
            modules,
        }
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

    fn run_external_function(&self, ext: &ExternalFn, args: Vec<Value>) -> EvalResult {
        let mut store = self.wasm_store.borrow_mut();

        let mut func_with_inst = None;
        for instance in &self.wasm_instances {
            if let Some(f) = instance.get_func(&mut *store, &ext.wasm_name) {
                func_with_inst = Some((f, *instance));
                break;
            }
        }

        let (func, func_instance) = func_with_inst.ok_or_else(|| {
            format!(
                "Wasm function {} not found in any loaded instance",
                ext.wasm_name
            )
        })?;

        let mut wasm_args = Vec::new();
        for v in args {
            match v {
                Value::Int(i) => wasm_args.push(Val::I64(i)),
                Value::Float(f) => wasm_args.push(Val::F64(f.to_bits())),
                Value::String(s) => {
                    let ptr = self.pass_string_to_wasm(&s, &mut *store, &func_instance)?;
                    wasm_args.push(Val::I32(ptr));
                    wasm_args.push(Val::I32(s.len() as i32));
                }
                _ => return Err("Unsupported FFI type".to_string()),
            }
        }

        let mut results = vec![Val::I64(0); func.ty(&mut *store).results().len()];
        func.call(&mut *store, &wasm_args, &mut results)
            .map_err(|e| format!("Wasm call failed: {}", e))?;

        if results.is_empty() {
            Ok(ExprResult::Normal(Value::Unit))
        } else {
            match results[0] {
                Val::I64(i) => Ok(ExprResult::Normal(Value::Int(i))),
                Val::F64(f) => Ok(ExprResult::Normal(Value::Float(f64::from_bits(f)))),
                Val::I32(i) => Ok(ExprResult::Normal(Value::Int(i as i64))),
                Val::F32(f) => Ok(ExprResult::Normal(Value::Float(f32::from_bits(f) as f64))),
                _ => Err("Unsupported Wasm return type".to_string()),
            }
        }
    }

    fn pass_string_to_wasm(
        &self,
        s: &str,
        store: &mut Store<WasiCtx>,
        instance: &Instance,
    ) -> Result<i32, String> {
        let alloc = instance
            .get_func(&mut *store, "allocate")
            .ok_or("Wasm instance must export 'allocate(i32) -> i32' to receive strings")?;

        let mut results = [Val::I32(0)];
        alloc
            .call(&mut *store, &[Val::I32(s.len() as i32)], &mut results)
            .map_err(|e| format!("allocate failed: {}", e))?;

        let ptr = match results[0] {
            Val::I32(p) => p,
            _ => return Err("allocate must return i32".to_string()),
        };

        let mem = instance
            .get_memory(&mut *store, "memory")
            .ok_or("Wasm instance must export 'memory'")?;

        mem.write(&mut *store, ptr as usize, s.as_bytes())
            .map_err(|e| format!("memory write failed: {}", e))?;

        Ok(ptr)
    }

    fn eval_body(&mut self, body: &[Spanned<Stmt>], env: &mut Env) -> EvalResult {
        for stmt in body {
            match &stmt.node {
                Stmt::Let {
                    name, sigil, value, ..
                } => {
                    let res = self.eval_expr(value, env)?;
                    match res {
                        ExprResult::Normal(val) => {
                            let final_val = if let Sigil::Mutable = sigil {
                                Value::Ref(Rc::new(RefCell::new(val)))
                            } else {
                                val
                            };
                            env.define(sigil.get_key(name), final_val);
                        }
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
                Stmt::Try {
                    body,
                    catch_param,
                    catch_body,
                } => {
                    let res = self.eval_body(body, env);
                    match res {
                        Ok(ExprResult::EarlyReturn(val)) => {
                            return Ok(ExprResult::EarlyReturn(val))
                        }
                        Ok(ExprResult::Normal(_)) => {}
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
                                    return Err(format!(
                                        "Cannot assign to immutable variable {}",
                                        name
                                    ));
                                }
                            } else {
                                return Err(format!("Variable {} not found", key));
                            }
                        }
                        Expr::Index(arr, idx) => {
                            let arr_res = self.eval_expr(arr, env)?;
                            let idx_res = self.eval_expr(idx, env)?;
                            match (arr_res, idx_res) {
                                (
                                    ExprResult::Normal(Value::Array(a)),
                                    ExprResult::Normal(Value::Int(i)),
                                ) => {
                                    let mut l = a.borrow_mut();
                                    let idx = i as usize;
                                    if idx < l.len() {
                                        l[idx] = val;
                                    } else {
                                        return Err(format!("Index out of bounds: {}", idx));
                                    }
                                }
                                (ExprResult::EarlyReturn(v), _)
                                | (_, ExprResult::EarlyReturn(v)) => {
                                    return Ok(ExprResult::EarlyReturn(v))
                                }
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
                            (Value::String(a), "++", Value::String(b)) => {
                                Ok(ExprResult::Normal(Value::String(a + &b)))
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
                            (Value::Float(a), "+.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a + b)))
                            }
                            (Value::Float(a), "-.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a - b)))
                            }
                            (Value::Float(a), "*.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a * b)))
                            }
                            (Value::Float(a), "/.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Float(a / b)))
                            }
                            (Value::Float(a), "==.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a == b)))
                            }
                            (Value::Float(a), "!=.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a != b)))
                            }
                            (Value::Float(a), "<.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a < b)))
                            }
                            (Value::Float(a), ">.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a > b)))
                            }
                            (Value::Float(a), "<=.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a <= b)))
                            }
                            (Value::Float(a), ">=.", Value::Float(b)) => {
                                Ok(ExprResult::Normal(Value::Bool(a >= b)))
                            }
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

                if let Some(val) = env.get(func) {
                    match val {
                        Value::NativeFunction(name) => {
                            if let Some(f) = self.native_functions.get(&name) {
                                return f(&evaluated_args);
                            } else {
                                return Err(format!("Native function '{}' not found", name));
                            }
                        }
                        Value::Function(name) => {
                            let res = self.run_function(&name, evaluated_args)?;
                            return Ok(ExprResult::Normal(res));
                        }
                        _ => {}
                    }
                }

                if let Some(ext) = self.external_functions.get(func).cloned() {
                    return self.run_external_function(&ext, evaluated_args);
                }

                if let Some(pos) = func.find('.') {
                    let mod_name = &func[..pos];
                    let item_name = &func[pos + 1..];

                    if let Some(sub_interp) = self.modules.get_mut(mod_name) {
                        let res = sub_interp.run_function(item_name, evaluated_args)?;
                        return Ok(ExprResult::Normal(res));
                    }

                    if let Some(handler) = self.handlers.get(mod_name).cloned() {
                        if let Some(target_func) =
                            handler.functions.iter().find(|f| f.name == item_name)
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

                // Fallback to global native function lookup (for stdlib if not in Env as var)
                if let Some(f) = self.native_functions.get(func) {
                    return f(&evaluated_args);
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
                Ok(ExprResult::Normal(Value::Array(Rc::new(RefCell::new(
                    vals,
                )))))
            }
            Expr::Index(arr, idx) => {
                let arr_res = self.eval_expr(arr, env)?;
                let idx_res = self.eval_expr(idx, env)?;
                match (arr_res, idx_res) {
                    (ExprResult::Normal(arr_val), ExprResult::Normal(Value::Int(i))) => {
                        let i = i as usize;
                        match arr_val {
                            Value::List(l) => {
                                if i < l.len() {
                                    Ok(ExprResult::Normal(l[i].clone()))
                                } else {
                                    Err(format!("Index out of bounds: {}", i))
                                }
                            }
                            Value::Array(a) => {
                                let l = a.borrow();
                                if i < l.len() {
                                    Ok(ExprResult::Normal(l[i].clone()))
                                } else {
                                    Err(format!("Index out of bounds: {}", i))
                                }
                            }
                            _ => Err("Cannot index non-array value".to_string()),
                        }
                    }
                    (ExprResult::EarlyReturn(v), _) | (_, ExprResult::EarlyReturn(v)) => {
                        Ok(ExprResult::EarlyReturn(v))
                    }
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

    fn match_pattern(
        &self,
        pattern: &Spanned<Pattern>,
        val: &Value,
    ) -> Option<HashMap<String, Value>> {
        match (&pattern.node, val) {
            (Pattern::Variable(name, _), v) => {
                let mut map = HashMap::new();
                map.insert(name.clone(), v.clone());
                Some(map)
            }
            (Pattern::Wildcard, _) => Some(HashMap::new()),
            (Pattern::Literal(lit), v) => match (lit, v) {
                (Literal::Int(a), Value::Int(b)) if a == b => Some(HashMap::new()),
                (Literal::Float(a), Value::Float(b)) if (a - b).abs() < f64::EPSILON => {
                    Some(HashMap::new())
                }
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
