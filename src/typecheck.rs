use crate::ast::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
}

impl From<(String, Span)> for TypeError {
    fn from((message, span): (String, Span)) -> Self {
        TypeError { message, span }
    }
}

#[derive(Clone)]
pub struct Scheme {
    pub vars: Vec<String>,
    pub typ: Type,
}

#[derive(Clone)]
pub struct TypeEnv {
    pub vars: HashMap<String, Scheme>,
    pub types: HashMap<String, TypeDef>,
    pub enums: HashMap<String, EnumDef>,
    pub linear_vars: HashSet<String>,
}

type Subst = HashMap<String, Type>;

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            vars: HashMap::new(),
            types: HashMap::new(),
            enums: HashMap::new(),
            linear_vars: HashSet::new(),
        }
    }

    pub fn insert(&mut self, name: String, scheme: Scheme) {
        if contains_linear(&scheme.typ) {
            self.linear_vars.insert(name.clone());
        }
        self.vars.insert(name, scheme);
    }

    pub fn get(&self, name: &str) -> Option<&Scheme> {
        self.vars.get(name)
    }

    pub fn apply(&mut self, subst: &Subst) {
        for scheme in self.vars.values_mut() {
            scheme.typ = apply_subst_type(subst, &scheme.typ);
        }
    }

    pub fn consume(&mut self, name: &str) -> Result<(), String> {
        if self.linear_vars.remove(name) {
            Ok(())
        } else if self.vars.contains_key(name) {
            if let Some(s) = self.vars.get(name) {
                if contains_linear(&s.typ) {
                    return Err(format!("Linear variable {} already consumed", name));
                }
            }
            Ok(())
        } else {
            Err(format!("Variable {} not found", name))
        }
    }
}

pub struct TypeChecker {
    pub supply: usize,
    pub env: TypeEnv,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut env = TypeEnv::new();
        let scheme_tx = Scheme { vars: vec![], typ: Type::UserDefined("Tx".to_string(), vec![]) };

        env.insert("db_driver.begin_tx".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![], Box::new(scheme_tx.typ.clone()), Box::new(Type::Row(vec![], None))) });
        env.insert("db_driver.commit".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![Type::UserDefined("Tx".to_string(), vec![])], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });
        env.insert("db_driver.rollback".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![Type::UserDefined("Tx".to_string(), vec![])], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });
        env.insert("log.info".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![Type::Str], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });
        env.insert("printf".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![Type::Str, Type::I64], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });
        env.insert("print_str".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![Type::Str], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });
        env.insert("print_i64".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![Type::I64], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });
        env.insert("drop_i64".to_string(), Scheme { vars: vec![], typ: Type::Arrow(vec![Type::Linear(Box::new(Type::I64))], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });
        env.insert("drop_array".to_string(), Scheme { vars: vec!["T".into()], typ: Type::Arrow(vec![Type::Array(Box::new(Type::Var("T".into())))], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))) });

        env.linear_vars.clear();
        TypeChecker { supply: 0, env }
    }

    fn new_var(&mut self) -> Type {
        let n = self.supply;
        self.supply += 1;
        Type::Var(format!("?{}", n))
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), TypeError> {
        for def in &program.definitions {
            match &def.node {
                TopLevel::TypeDef(td) => { self.env.types.insert(td.name.clone(), td.clone()); }
                TopLevel::Enum(ed) => {
                    self.env.enums.insert(ed.name.clone(), ed.clone());
                    for v in &ed.variants {
                        if v.fields.is_empty() {
                            let mut targs = Vec::new(); for p in &ed.type_params { targs.push(Type::Var(p.clone())); }
                            self.env.insert(v.name.clone(), Scheme { vars: ed.type_params.clone(), typ: Type::UserDefined(ed.name.clone(), targs) });
                        }
                    }
                }
                TopLevel::Port(port) => {
                    for sig in &port.functions {
                        let name = format!("{}.{}", port.name, sig.name);
                        let ptypes: Vec<Type> = sig.params.iter().map(|p| p.typ.clone()).collect();
                        self.env.insert(name, Scheme { vars: vec![], typ: Type::Arrow(ptypes, Box::new(sig.ret_type.clone()), Box::new(sig.effects.clone())) });
                    }
                }
                TopLevel::Function(func) => { self.env.insert(func.name.clone(), self.generalize_top_level(func)); }
                _ => {}
            }
        }
        self.env.linear_vars.clear();

        for def in &program.definitions {
            match &def.node {
                TopLevel::Function(func) => {
                    if func.name == "main" && func.is_public { return Err(TypeError { message: "main function must be private (remove 'pub')".into(), span: def.span.clone() }); }
                    self.check_function(func, &def.span)?;
                }
                TopLevel::Handler(h) => { self.check_handler(h, &def.span)?; }
                _ => {}
            }
        }
        Ok(())
    }

    fn generalize_top_level(&self, func: &Function) -> Scheme {
        let vars: HashSet<String> = func.type_params.iter().cloned().collect();
        let pts: Vec<Type> = func.params.iter().map(|p| self.convert_user_defined_to_var(&p.typ, &vars)).collect();
        let rt = self.convert_user_defined_to_var(&func.ret_type, &vars);
        let ef = self.convert_user_defined_to_var(&func.effects, &vars);
        Scheme { vars: func.type_params.clone(), typ: Type::Arrow(pts, Box::new(rt), Box::new(ef)) }
    }

    fn convert_user_defined_to_var(&self, typ: &Type, vars: &HashSet<String>) -> Type {
        match typ {
            Type::UserDefined(n, args) => { if args.is_empty() && vars.contains(n) { Type::Var(n.clone()) } else { Type::UserDefined(n.clone(), args.iter().map(|a| self.convert_user_defined_to_var(a, vars)).collect()) } }
            Type::Arrow(p, r, e) => Type::Arrow(p.iter().map(|t| self.convert_user_defined_to_var(t, vars)).collect(), Box::new(self.convert_user_defined_to_var(r, vars)), Box::new(self.convert_user_defined_to_var(e, vars))),
            Type::Result(o, e) => Type::Result(Box::new(self.convert_user_defined_to_var(o, vars)), Box::new(self.convert_user_defined_to_var(e, vars))),
            Type::Ref(i) => Type::Ref(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Linear(i) => Type::Linear(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Borrow(i) => Type::Borrow(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::List(i) => Type::List(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Row(es, t) => Type::Row(es.iter().map(|x| self.convert_user_defined_to_var(x, vars)).collect(), t.as_ref().map(|x| Box::new(self.convert_user_defined_to_var(x, vars)))),
            Type::Record(fs) => Type::Record(fs.iter().map(|(n, t)| (n.clone(), self.convert_user_defined_to_var(t, vars))).collect()),
            _ => typ.clone(),
        }
    }

    fn check_function(&mut self, func: &Function, span: &Span) -> Result<(), TypeError> {
        let mut env = self.env.clone();
        for p in &func.params { env.insert(p.sigil.get_key(&p.name), Scheme { vars: vec![], typ: p.typ.clone() }); }
        if contains_ref(&func.ret_type) { return Err(TypeError { message: "Cannot return Ref".into(), span: span.clone() }); }
        self.infer_body(&func.body, &mut env, &func.ret_type, &func.effects)?;
        if !env.linear_vars.is_empty() { return Err(TypeError { message: format!("Unused linear: {:?}", env.linear_vars), span: span.clone() }); }
        Ok(())
    }

    fn check_handler(&mut self, h: &Handler, span: &Span) -> Result<(), TypeError> {
        for f in &h.functions {
            let name = format!("{}.{}", h.port_name, f.name);
            if let Some(sch) = self.env.get(&name).cloned() {
                self.unify(&sch.typ, &self.generalize_top_level(f).typ).map_err(|e| TypeError { message: e, span: span.clone() })?;
                self.check_function(f, span)?;
            } else { return Err(TypeError { message: format!("Fn {} not in port", f.name), span: span.clone() }); }
        }
        Ok(())
    }

    fn infer_body(&mut self, body: &[Spanned<Stmt>], env: &mut TypeEnv, er: &Type, ee: &Type) -> Result<(), TypeError> {
        for s in body {
            match &s.node {
                Stmt::Let { name, sigil, typ, value } => {
                    let (s1, t1) = self.infer(env, value, er, ee)?; env.apply(&s1);
                    if let Some(ann) = typ {
                        let sa = self.unify(&t1, ann).map_err(|e| TypeError { message: e, span: value.span.clone() })?; env.apply(&sa);
                    }
                    let ft = match sigil {
                        Sigil::Mutable => { if contains_linear(&t1) { return Err(TypeError { message: "Mutable linear".into(), span: value.span.clone() }); } Type::Ref(Box::new(t1)) }
                        Sigil::Linear => {
                            if contains_linear(&t1) { t1 }
                            else { Type::Linear(Box::new(t1)) }
                        },
                        Sigil::Immutable => { if contains_ref(&t1) { return Err(TypeError { message: "Immutable Ref".into(), span: value.span.clone() }); } t1 }
                    };
                    env.insert(sigil.get_key(name), self.generalize(env, ft));
                }
                Stmt::Return(e) => {
                    let (s1, t1) = self.infer(env, e, er, ee)?; env.apply(&s1);
                    if !env.linear_vars.is_empty() { return Err(TypeError { message: "Unused linear".into(), span: e.span.clone() }); }
                    self.unify(&t1, &apply_subst_type(&s1, er)).map_err(|err| TypeError { message: err, span: e.span.clone() })?;
                }
                Stmt::Expr(e) => { self.infer(env, e, er, ee)?; }
                Stmt::Assign { target, value } => {
                    let (s_v, t_v) = self.infer(env, value, er, ee)?; env.apply(&s_v);
                    match &target.node {
                        Expr::Variable(name, sigil) => {
                            if let Sigil::Immutable = sigil { return Err(TypeError { message: "Mutating immutable".into(), span: s.span.clone() }); }
                            if let Some(sch) = env.get(&sigil.get_key(name)) {
                                if let Type::Ref(i) = self.instantiate(sch) { self.unify(&t_v, &i).map_err(|e| TypeError { message: e, span: value.span.clone() })?; }
                                else { return Err(TypeError { message: "Not a ref".into(), span: s.span.clone() }); }
                            } else { return Err(TypeError { message: "Not found".into(), span: s.span.clone() }); }
                        }
                        Expr::Index(arr, idx) => {
                             // Typecheck index
                             let (s_idx, t_idx) = self.infer(env, idx, er, ee)?;
                             env.apply(&s_idx);
                             self.unify(&t_idx, &Type::I64).map_err(|e| TypeError { message: e, span: idx.span.clone() })?;

                             // Typecheck array WITHOUT consuming if it's a variable
                             let t_arr = match &arr.node {
                                 Expr::Variable(n, s) => {
                                     let key = s.get_key(n);
                                     if let Some(sch) = env.get(&key) { self.instantiate(sch) }
                                     else { return Err(TypeError { message: format!("Not found: {}", key), span: arr.span.clone() }); }
                                 }
                                 _ => {
                                     let (s_a, t_a) = self.infer(env, arr, er, ee)?;
                                     env.apply(&s_a); t_a
                                 }
                             };

                             let t_arr_unwrapped = match t_arr {
                                 Type::Linear(inner) => *inner,
                                 other => other,
                             };

                             let elem_t = match t_arr_unwrapped {
                                 Type::Array(t) => *t,
                                 Type::Borrow(inner) => match *inner {
                                     Type::Array(t) => *t,
                                     _ => return Err(TypeError { message: "Not an array".into(), span: arr.span.clone() }),
                                 }
                                 _ => return Err(TypeError { message: "Not an array".into(), span: arr.span.clone() }),
                             };
                             self.unify(&t_v, &elem_t).map_err(|e| TypeError { message: e, span: value.span.clone() })?;
                        }
                        _ => return Err(TypeError { message: "Invalid assignment target".into(), span: s.span.clone() }),
                    }
                }
                Stmt::Conc(ts) => { for t in ts { self.check_task(t, env, &s.span)?; } }
                Stmt::Try { body, catch_param, catch_body } => {
                    let exn = Type::UserDefined("Exn".into(), vec![]);
                    let try_eff = Type::Row(vec![exn], Some(Box::new(ee.clone())));
                    let mut et = env.clone(); self.infer_body(body, &mut et, er, &try_eff)?;
                    let mut ec = env.clone(); ec.insert(catch_param.clone(), Scheme { vars: vec![], typ: Type::Str });
                    self.infer_body(catch_body, &mut ec, er, ee)?;
                    if et.linear_vars != ec.linear_vars { return Err(TypeError { message: "Linear mismatch".into(), span: s.span.clone() }); }
                    env.linear_vars = et.linear_vars;
                }
                Stmt::Comment => {}
            }
        }
        Ok(())
    }

    fn check_task(&mut self, t: &Function, oe: &TypeEnv, _span: &Span) -> Result<(), TypeError> {
        let mut te = TypeEnv::new(); te.types = oe.types.clone(); te.enums = oe.enums.clone();
        for (k, s) in &oe.vars { if !k.starts_with('~') { te.insert(k.clone(), s.clone()); } }
        self.infer_body(&t.body, &mut te, &Type::Unit, &Type::Row(vec![], None))
    }

    pub fn check_repl_stmt(&mut self, s: &Spanned<Stmt>) -> Result<Type, TypeError> {
        let mut env = std::mem::replace(&mut self.env, TypeEnv::new());
        let res = (|| {
            let ev = self.new_var();
            match &s.node {
                Stmt::Expr(e) => { let (sub, t) = self.infer(&mut env, e, &Type::Unit, &ev)?; env.apply(&sub); Ok(t) }
                _ => { self.infer_body(&[s.clone()], &mut env, &Type::Unit, &ev)?; Ok(Type::Unit) }
            }
        })();
        self.env = env; res
    }

    fn infer(&mut self, env: &mut TypeEnv, e: &Spanned<Expr>, er: &Type, ee: &Type) -> Result<(Subst, Type), TypeError> {
        match &e.node {
            Expr::Literal(l) => Ok((HashMap::new(), match l {
                Literal::Int(_) => Type::I64,
                Literal::Float(_) => Type::Float,
                Literal::Bool(_) => Type::Bool,
                Literal::String(_) => Type::Str,
                Literal::Unit => Type::Unit,
            })),
            Expr::Variable(n, s) => {
                let key = s.get_key(n);
                if let Some(sch) = env.get(&key).cloned() {
                    let mut t = self.instantiate(&sch);
                    if let Sigil::Mutable = s { if let Type::Ref(i) = t { t = *i; } }
                    if let Type::Borrow(i) = t { t = *i; }
                    else if contains_linear(&t) { env.consume(&key).map_err(|m| TypeError { message: m, span: e.span.clone() })?; }
                    Ok((HashMap::new(), t))
                } else { Err(TypeError { message: format!("Not found: {}", key), span: e.span.clone() }) }
            }
            Expr::BinaryOp(l, op, r) => {
                let (s1, t1) = self.infer(env, l, er, ee)?;
                let (s2, t2) = self.infer(env, r, er, ee)?;
                let mut s = compose_subst(&s1, &s2);
                match op.as_str() {
                    "+" | "-" | "*" | "/" => {
                        let s3 = self.unify(&apply_subst_type(&s, &t1), &Type::I64).map_err(|m| TypeError { message: m, span: l.span.clone() })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self.unify(&apply_subst_type(&s, &t2), &Type::I64).map_err(|m| TypeError { message: m, span: r.span.clone() })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::I64))
                    }
                    "+." | "-." | "*." | "/." => {
                        let s3 = self.unify(&apply_subst_type(&s, &t1), &Type::Float).map_err(|m| TypeError { message: m, span: l.span.clone() })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self.unify(&apply_subst_type(&s, &t2), &Type::Float).map_err(|m| TypeError { message: m, span: r.span.clone() })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Float))
                    }
                    "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                        let s3 = self.unify(&apply_subst_type(&s, &t1), &Type::I64).map_err(|m| TypeError { message: m, span: l.span.clone() })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self.unify(&apply_subst_type(&s, &t2), &Type::I64).map_err(|m| TypeError { message: m, span: r.span.clone() })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Bool))
                    }
                    "==." | "!=." | "<." | ">." | "<=." | ">=." => {
                        let s3 = self.unify(&apply_subst_type(&s, &t1), &Type::Float).map_err(|m| TypeError { message: m, span: l.span.clone() })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self.unify(&apply_subst_type(&s, &t2), &Type::Float).map_err(|m| TypeError { message: m, span: r.span.clone() })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Bool))
                    }
                    _ => Err(TypeError { message: format!("Unknown op {}", op), span: e.span.clone() }),
                }
            }
            Expr::Borrow(n, s) => {
                if let Some(sch) = env.get(&s.get_key(n)).cloned() {
                    let t = self.instantiate(&sch);
                    let i = match t { Type::Linear(u) | Type::Borrow(u) => *u, o => o };
                    Ok((HashMap::new(), Type::Borrow(Box::new(i))))
                } else { Err(TypeError { message: "Not found".into(), span: e.span.clone() }) }
            }
            Expr::Call { func, args, perform } => {
                let (mut s, ft) = if let Some(sch) = env.get(func).cloned() { (HashMap::new(), self.instantiate(&sch)) }
                else { return Err(TypeError { message: format!("Fn {} not found", func), span: e.span.clone() }); };
                let rt = self.new_var(); let pts: Vec<Type> = args.iter().map(|_| self.new_var()).collect(); let ec = self.new_var();
                let sf = self.unify(&ft, &Type::Arrow(pts.clone(), Box::new(rt.clone()), Box::new(ec.clone()))).map_err(|m| TypeError { message: m, span: e.span.clone() })?;
                s = compose_subst(&s, &sf);
                let eci = apply_subst_type(&s, &ec);
                let eco = match eci { Type::Row(el, None) => Type::Row(el, Some(Box::new(self.new_var()))), Type::Unit => Type::Row(vec![], Some(Box::new(self.new_var()))), o => o };
                let se = self.unify(&apply_subst_type(&s, ee), &eco).map_err(|m| TypeError { message: m, span: e.span.clone() })?;
                s = compose_subst(&s, &se);

                // Enforce perform
                let actual_eff = apply_subst_type(&s, &ec);
                let is_pure = match actual_eff {
                    Type::Row(effs, tail) => effs.is_empty() && tail.is_none(),
                    Type::Unit => true,
                    _ => false,
                };
                if !perform && !is_pure {
                    return Err(TypeError { message: "Effectful call requires 'perform'".into(), span: e.span.clone() });
                }

                for (pt, (_, ae)) in pts.iter().zip(args) {
                    let (sa, ta) = self.infer(env, ae, er, ee)?; s = compose_subst(&s, &sa);
                    let su = self.unify(&ta, &apply_subst_type(&s, pt)).map_err(|m| TypeError { message: m, span: ae.span.clone() })?;
                    s = compose_subst(&s, &su);
                }
                Ok((s.clone(), apply_subst_type(&s, &rt)))
            }
            Expr::Constructor(name, args) => {
                if name == "Ok" && args.len() == 1 { let (s, t) = self.infer(env, &args[0], er, ee)?; return Ok((s, Type::Result(Box::new(t), Box::new(self.new_var())))); }
                if name == "Err" && args.len() == 1 { let (s, t) = self.infer(env, &args[0], er, ee)?; return Ok((s, Type::Result(Box::new(self.new_var()), Box::new(t)))); }
                for ed in env.enums.values().cloned() {
                    if let Some(v) = ed.variants.iter().find(|x| x.name == *name) {
                        if v.fields.len() != args.len() { return Err(TypeError { message: "Arity mismatch".into(), span: e.span.clone() }); }
                        let mut s = HashMap::new(); let targs: Vec<Type> = ed.type_params.iter().map(|_| self.new_var()).collect();
                        let mut inst = HashMap::new(); for (p, a) in ed.type_params.iter().zip(&targs) { inst.insert(p.clone(), a.clone()); }
                        for (ft, ae) in v.fields.iter().zip(args) {
                            let (sa, ta) = self.infer(env, ae, er, ee)?; s = compose_subst(&s, &sa);
                            let su = self.unify(&ta, &apply_subst_type(&s, &apply_subst_type(&inst, ft))).map_err(|m| TypeError { message: m, span: ae.span.clone() })?;
                            s = compose_subst(&s, &su);
                        }
                        return Ok((s.clone(), Type::UserDefined(ed.name.clone(), targs.iter().map(|a| apply_subst_type(&s, a)).collect())));
                    }
                }
                Ok((HashMap::new(), Type::Unit))
            }
            Expr::Record(fs) => {
                let mut s = HashMap::new(); let mut rfs = Vec::new();
                for (n, ex) in fs { let (sa, ta) = self.infer(env, ex, er, ee)?; s = compose_subst(&s, &sa); rfs.push((n.clone(), ta)); }
                Ok((s, Type::Record(rfs)))
            }
            Expr::List(exprs) => {
                let elem_type = self.new_var();
                let mut s = HashMap::new();
                for ex in exprs {
                    let (s_ex, t_ex) = self.infer(env, ex, er, ee)?;
                    s = compose_subst(&s, &s_ex);
                    let s_unify = self.unify(&t_ex, &apply_subst_type(&s, &elem_type)).map_err(|m| TypeError { message: m, span: ex.span.clone() })?;
                    s = compose_subst(&s, &s_unify);
                }
                let final_elem_type = apply_subst_type(&s, &elem_type);
                if contains_ref(&final_elem_type) {
                     return Err(TypeError { message: "List cannot contain References".into(), span: e.span.clone() });
                }
                Ok((s, Type::List(Box::new(final_elem_type))))
            }
            Expr::Array(exprs) => {
                let elem_type = self.new_var();
                let mut s = HashMap::new();
                for ex in exprs {
                    let (s_ex, t_ex) = self.infer(env, ex, er, ee)?;
                    s = compose_subst(&s, &s_ex);
                    let s_unify = self.unify(&t_ex, &apply_subst_type(&s, &elem_type)).map_err(|m| TypeError { message: m, span: ex.span.clone() })?;
                    s = compose_subst(&s, &s_unify);
                }
                let final_elem_type = apply_subst_type(&s, &elem_type);
                if contains_ref(&final_elem_type) {
                     return Err(TypeError { message: "Array cannot contain References".into(), span: e.span.clone() });
                }
                Ok((s, Type::Array(Box::new(final_elem_type))))
            }
            Expr::Index(arr, idx) => {
                let (s1, t_arr) = self.infer(env, arr, er, ee)?;
                let (s2, t_idx) = self.infer(env, idx, er, ee)?;
                let mut s = compose_subst(&s1, &s2);
                let s_idx = self.unify(&t_idx, &Type::I64).map_err(|m| TypeError { message: m, span: idx.span.clone() })?;
                s = compose_subst(&s, &s_idx);
                
                let t_arr_inst = apply_subst_type(&s, &t_arr);
                let t_arr_unwrapped = match t_arr_inst {
                    Type::Linear(inner) => *inner,
                    other => other,
                };

                let elem_t = match &t_arr_unwrapped {
                    Type::Array(t) => (**t).clone(),
                    Type::Borrow(inner) => match &**inner {
                        Type::Array(t) => (**t).clone(),
                        _ => return Err(TypeError { message: "Indexing non-array".into(), span: arr.span.clone() }),
                    },
                    _ => {
                        let et = self.new_var();
                        let su = self.unify(&t_arr_unwrapped, &Type::Array(Box::new(et.clone()))).map_err(|m| TypeError { message: m, span: arr.span.clone() })?;
                        s = compose_subst(&s, &su);
                        apply_subst_type(&s, &et)
                    }
                };

                if contains_linear(&elem_t) {
                     return Err(TypeError { message: "Cannot move linear element out of array".into(), span: e.span.clone() });
                }
                Ok((s, elem_t))
            }
            Expr::FieldAccess(rec, fnm) => {
                let (s1, tr) = self.infer(env, rec, er, ee)?; let tr = apply_subst_type(&s1, &tr);
                if let Type::Record(fs) = &tr { if let Some((_, t)) = fs.iter().find(|(n, _)| n == fnm) { return Ok((s1, t.clone())); } }
                if let Type::UserDefined(tn, ta) = &tr {
                    if let Some(td) = env.types.get(tn).cloned() {
                        if let Some((_, ft)) = td.fields.iter().find(|(n, _)| n == fnm) {
                            let mut su = HashMap::new(); for (p, a) in td.type_params.iter().zip(ta) { su.insert(p.clone(), a.clone()); }
                            return Ok((s1, apply_subst_type(&su, ft)));
                        }
                    }
                }
                Err(TypeError { message: format!("Field {} not found", fnm), span: e.span.clone() })
            }
            Expr::If { cond, then_branch, else_branch } => {
                let (s1, tc) = self.infer(env, cond, er, ee)?;
                let s = compose_subst(&s1, &self.unify(&tc, &Type::Bool).map_err(|m| TypeError { message: m, span: cond.span.clone() })?);
                let mut et = env.clone(); et.apply(&s); self.infer_body(then_branch, &mut et, er, ee)?;
                let mut ee_env = env.clone(); ee_env.apply(&s); if let Some(eb) = else_branch { self.infer_body(eb, &mut ee_env, er, ee)?; }
                if et.linear_vars != ee_env.linear_vars { return Err(TypeError { message: "Linear mismatch".into(), span: e.span.clone() }); }
                env.linear_vars = et.linear_vars; Ok((s, Type::Unit))
            }
            Expr::Match { target, cases } => {
                let (s1, tt) = self.infer(env, target, er, ee)?; let mut s = s1;
                self.check_exhaustiveness(&apply_subst_type(&s, &tt), cases).map_err(|m| TypeError { message: m, span: e.span.clone() })?;
                let mut rv: Option<HashSet<String>> = None;
                for case in cases {
                    let mut le = env.clone(); le.apply(&s);
                    let sm = self.bind_pattern(&case.pattern, &apply_subst_type(&s, &tt), &mut le)?;
                    s = compose_subst(&s, &sm); le.apply(&sm); self.infer_body(&case.body, &mut le, er, ee)?;
                    if let Some(p) = &rv { if p != &le.linear_vars { return Err(TypeError { message: "Linear mismatch".into(), span: case.pattern.span.clone() }); } }
                    else { rv = Some(le.linear_vars.clone()); }
                }
                if let Some(vars) = rv { env.linear_vars = vars; }
                Ok((s, Type::Unit))
            }
            Expr::Raise(ex) => {
                let (s, t) = self.infer(env, ex, er, ee)?;
                let ss = self.unify(&t, &Type::Str).map_err(|m| TypeError { message: m, span: ex.span.clone() })?;
                let mut s = compose_subst(&s, &ss);
                let exn_type = Type::UserDefined("Exn".into(), vec![]);
                let required_eff = Type::Row(vec![exn_type], Some(Box::new(self.new_var())));
                let s_eff = self.unify(&apply_subst_type(&s, ee), &required_eff).map_err(|_| TypeError { message: "raise requires 'Exn'".into(), span: e.span.clone() })?;
                s = compose_subst(&s, &s_eff);
                Ok((s, self.new_var()))
            }
        }
    }

    fn bind_pattern(&mut self, p: &Spanned<Pattern>, tt: &Type, env: &mut TypeEnv) -> Result<Subst, TypeError> {
        match &p.node {
            Pattern::Variable(n, _) => { env.insert(n.clone(), Scheme { vars: vec![], typ: tt.clone() }); Ok(HashMap::new()) }
            Pattern::Constructor(n, pats) => {
                if n == "Ok" && pats.len() == 1 {
                    let (tok, terr) = (self.new_var(), self.new_var());
                    let s = self.unify(tt, &Type::Result(Box::new(tok.clone()), Box::new(terr))).map_err(|m| TypeError { message: m, span: p.span.clone() })?;
                    let sp = self.bind_pattern(&pats[0], &apply_subst_type(&s, &tok), env)?; Ok(compose_subst(&s, &sp))
                } else if n == "Err" && pats.len() == 1 {
                    let (tok, terr) = (self.new_var(), self.new_var());
                    let s = self.unify(tt, &Type::Result(Box::new(tok), Box::new(terr.clone()))).map_err(|m| TypeError { message: m, span: p.span.clone() })?;
                    let sp = self.bind_pattern(&pats[0], &apply_subst_type(&s, &terr), env)?; Ok(compose_subst(&s, &sp))
                } else {
                    for ed in env.enums.values().cloned() {
                        if let Some(v) = ed.variants.iter().find(|x| x.name == *n) {
                            if v.fields.len() != pats.len() { return Err(TypeError { message: "Arity mismatch".into(), span: p.span.clone() }); }
                            let targs: Vec<Type> = ed.type_params.iter().map(|_| self.new_var()).collect();
                            let s_en = self.unify(tt, &Type::UserDefined(ed.name.clone(), targs.clone())).map_err(|m| TypeError { message: m, span: p.span.clone() })?;
                            let mut subst = s_en; let mut inst = HashMap::new(); for (pa, a) in ed.type_params.iter().zip(targs) { inst.insert(pa.clone(), a.clone()); }
                            for (ft, pt) in v.fields.iter().zip(pats) {
                                let sp = self.bind_pattern(pt, &apply_subst_type(&subst, &apply_subst_type(&inst, ft)), env)?;
                                subst = compose_subst(&subst, &sp);
                            }
                            return Ok(subst);
                        }
                    }
                    Err(TypeError { message: format!("Unknown ctor {}", n), span: p.span.clone() })
                }
            }
            Pattern::Literal(l) => {
                let tl = match l { Literal::Int(_) => Type::I64, Literal::Float(_) => Type::Float, Literal::Bool(_) => Type::Bool, Literal::String(_) => Type::Str, Literal::Unit => Type::Unit };
                self.unify(tt, &tl).map_err(|m| TypeError { message: m, span: p.span.clone() })
            }
            Pattern::Wildcard => { if contains_linear(tt) { return Err(TypeError { message: "Discard linear".into(), span: p.span.clone() }); } Ok(HashMap::new()) }
            Pattern::Record(pfs, open) => {
                let tfs = match tt {
                    Type::Record(fs) => { let mut m = HashMap::new(); for (n,t) in fs { m.insert(n.clone(), t.clone()); } m }
                    Type::UserDefined(n, args) => {
                        if let Some(td) = env.types.get(n).cloned() {
                            let mut m = HashMap::new(); let mut su = HashMap::new(); for (pa, a) in td.type_params.iter().zip(args) { su.insert(pa.clone(), a.clone()); }
                            for (nm, t) in &td.fields { m.insert(nm.clone(), apply_subst_type(&su, t)); } m
                        } else { return Err(TypeError { message: "Unknown type".into(), span: p.span.clone() }); }
                    }
                    _ => return Err(TypeError { message: "Not record".into(), span: p.span.clone() }),
                };
                let mut sub = HashMap::new(); let mut matched = HashSet::new();
                for (n, pt) in pfs {
                    if let Some(tf) = tfs.get(n) {
                        let sp = self.bind_pattern(pt, &apply_subst_type(&sub, tf), env)?; sub = compose_subst(&sub, &sp); matched.insert(n.clone());
                    } else { return Err(TypeError { message: format!("No field {}", n), span: pt.span.clone() }); }
                }
                if !open { for k in tfs.keys() { if !matched.contains(k) { return Err(TypeError { message: format!("Missing {}", k), span: p.span.clone() }); } } }
                Ok(sub)
            }
        }
    }

    fn instantiate(&mut self, scheme: &Scheme) -> Type {
        let mut subst = HashMap::new();
        for var in &scheme.vars { subst.insert(var.clone(), self.new_var()); }
        apply_subst_type(&subst, &scheme.typ)
    }

    fn check_exhaustiveness(&self, tt: &Type, cases: &[MatchCase]) -> Result<(), String> {
        let matrix: Vec<Vec<PatRef>> = cases.iter().map(|c| vec![PatRef::Original(&c.pattern)]).collect();
        self.check_matrix(&matrix, &[tt])
    }

    fn check_matrix(&self, matrix: &[Vec<PatRef>], types: &[&Type]) -> Result<(), String> {
        if matrix.is_empty() { return Err("Non-exhaustive".to_string()); }
        if types.is_empty() { return Ok(()); }
        let ft = types[0]; let rt = &types[1..];
        match ft {
            Type::Bool => { self.check_constructor_matrix(matrix, "true", 0, &[], rt)?; self.check_constructor_matrix(matrix, "false", 0, &[], rt)?; Ok(()) }
            Type::Result(ok, err) => { self.check_constructor_matrix(matrix, "Ok", 1, &[&**ok], rt)?; self.check_constructor_matrix(matrix, "Err", 1, &[&**err], rt)?; Ok(()) }
            Type::Record(fields) => {
                let mut fts: Vec<&Type> = fields.iter().map(|(_, t)| t).collect(); fts.extend_from_slice(rt);
                let mut nm = Vec::new();
                for row in matrix {
                    let p = &row[0]; let rr = &row[1..];
                    match p.node() {
                        Pattern::Record(pfs, open) => {
                            let mut nr = Vec::new();
                            for (fnm, _) in fields {
                                if let Some((_, pt)) = pfs.iter().find(|(n, _)| n == fnm) { nr.push(PatRef::Original(pt)); }
                                else if *open { nr.push(PatRef::Synthetic(Pattern::Wildcard, p.span())); }
                                else { nr.push(PatRef::Synthetic(Pattern::Wildcard, p.span())); }
                            }
                            nr.extend_from_slice(rr); nm.push(nr);
                        }
                        Pattern::Wildcard | Pattern::Variable(..) => {
                            let mut nr = vec![PatRef::Synthetic(Pattern::Wildcard, p.span()); fields.len()];
                            nr.extend_from_slice(rr); nm.push(nr);
                        }
                        _ => {}
                    }
                }
                self.check_matrix(&nm, &fts)
            }
            Type::Unit => { self.check_constructor_matrix(matrix, "()", 0, &[], rt)?; Ok(()) }
            Type::UserDefined(name, args) => {
                 if let Some(ed) = self.env.enums.get(name).cloned() {
                     let mut subst = HashMap::new(); for (p, a) in ed.type_params.iter().zip(args) { subst.insert(p.clone(), a.clone()); }
                     for v in &ed.variants {
                         let ats: Vec<Type> = v.fields.iter().map(|f| apply_subst_type(&subst, f)).collect();
                         let ars: Vec<&Type> = ats.iter().collect();
                         self.check_constructor_matrix(matrix, &v.name, v.fields.len(), &ars, rt)?;
                     }
                     Ok(())
                 } else { self.check_wildcard_matrix(matrix, rt) }
            }
            _ => self.check_wildcard_matrix(matrix, rt)
        }
    }

    fn check_wildcard_matrix(&self, matrix: &[Vec<PatRef>], rt: &[&Type]) -> Result<(), String> {
        let mut nm = Vec::new();
        for row in matrix { match row[0].node() { Pattern::Wildcard | Pattern::Variable(..) => nm.push(row[1..].to_vec()), _ => {} } }
        if nm.is_empty() { return Err("Non-exhaustive".to_string()); }
        self.check_matrix(&nm, rt)
    }

    fn check_constructor_matrix(&self, matrix: &[Vec<PatRef>], ctor: &str, arity: usize, ats: &[&Type], rt: &[&Type]) -> Result<(), String> {
        let mut nm = Vec::new(); let mut nt = ats.to_vec(); nt.extend_from_slice(rt);
        for row in matrix {
            let p = &row[0]; let rest = &row[1..];
            match p.node() {
                 Pattern::Constructor(c, args) => if c == ctor { let mut nr: Vec<PatRef> = args.iter().map(|a| PatRef::Original(a)).collect(); nr.extend_from_slice(rest); nm.push(nr); },
                 Pattern::Literal(lit) => {
                     let name = match lit { Literal::Bool(true) => "true", Literal::Bool(false) => "false", Literal::Unit => "()", _ => "" };
                     if name == ctor { let mut nr = Vec::new(); nr.extend_from_slice(rest); nm.push(nr); }
                 },
                 Pattern::Wildcard | Pattern::Variable(..) => {
                     let mut nr = vec![PatRef::Synthetic(Pattern::Wildcard, p.span()); arity]; nr.extend_from_slice(rest); nm.push(nr);
                 },
                 _ => {}
            }
        }
        self.check_matrix(&nm, &nt).map_err(|_| format!("Missing {}", ctor))
    }

    fn generalize(&self, env: &TypeEnv, typ: Type) -> Scheme {
        let evs = get_free_vars_env(env); let tvs = get_free_vars_type(&typ);
        let free: Vec<String> = tvs.difference(&evs).cloned().collect(); Scheme { vars: free, typ }
    }

    fn unify(&mut self, t1: &Type, t2: &Type) -> Result<Subst, String> {
        match (t1, t2) {
            (t1, t2) if t1 == t2 => Ok(HashMap::new()),
            (Type::Var(n), t) | (t, Type::Var(n)) => { if occurs_check(n, t) { return Err("Recursive".into()); } let mut s = HashMap::new(); s.insert(n.clone(), t.clone()); Ok(s) }
            (Type::Arrow(p1, r1, e1), Type::Arrow(p2, r2, e2)) => {
                if p1.len() != p2.len() { return Err("Arity mismatch".into()); }
                let mut s = HashMap::new();
                for (a, b) in p1.iter().zip(p2) { let sn = self.unify(&apply_subst_type(&s, a), &apply_subst_type(&s, b))?; s = compose_subst(&s, &sn); }
                let sr = self.unify(&apply_subst_type(&s, r1), &apply_subst_type(&s, r2))?; s = compose_subst(&s, &sr);
                let se = self.unify(&apply_subst_type(&s, e1), &apply_subst_type(&s, e2))?; s = compose_subst(&s, &se);
                Ok(s)
            }
            (Type::Row(e1, t1), Type::Row(e2, t2)) => {
                let mut s = HashMap::new(); let mut e2r = e2.clone(); let mut e1r = Vec::new();
                for h1 in e1 { let h1s = apply_subst_type(&s, h1); if let Some(idx) = e2r.iter().position(|h2| h1s == apply_subst_type(&s, h2)) { e2r.remove(idx); } else { e1r.push(h1.clone()); } }
                let ft = if t1.is_some() || t2.is_some() || !e1r.is_empty() || !e2r.is_empty() { Some(Box::new(self.new_var())) } else { None };
                if let Some(i) = t1 { let row = Type::Row(e2r, ft.clone()); let sn = self.unify(&apply_subst_type(&s, i), &row)?; s = compose_subst(&s, &sn); }
                else if !e2r.is_empty() { return Err("Row mismatch".into()); }
                if let Some(i) = t2 { let row = Type::Row(e1r, ft); let sn = self.unify(&apply_subst_type(&s, i), &row)?; s = compose_subst(&s, &sn); }
                else if !e1r.is_empty() { return Err("Row mismatch".into()); }
                Ok(s)
            }
            (Type::Record(f1), Type::Record(f2)) => {
                if f1.len() != f2.len() { return Err("Arity mismatch".into()); }
                let (mut f1s, mut f2s) = (f1.clone(), f2.clone()); f1s.sort_by(|a,b| a.0.cmp(&b.0)); f2s.sort_by(|a,b| a.0.cmp(&b.0));
                let mut s = HashMap::new();
                for ((n1, t1), (n2, t2)) in f1s.iter().zip(f2s.iter()) { if n1 != n2 { return Err("Field mismatch".into()); } let sn = self.unify(&apply_subst_type(&s, t1), &apply_subst_type(&s, t2))?; s = compose_subst(&s, &sn); }
                Ok(s)
            }
            (Type::UserDefined(n1, a1), Type::UserDefined(n2, a2)) if n1 == n2 => {
                if a1.len() != a2.len() { return Err("Arity mismatch".into()); }
                let mut s = HashMap::new(); for (a, b) in a1.iter().zip(a2) { let sn = self.unify(&apply_subst_type(&s, a), &apply_subst_type(&s, b))?; s = compose_subst(&s, &sn); }
                Ok(s)
            }
            (Type::List(i1), Type::List(i2)) => self.unify(i1, i2),
            (Type::Array(i1), Type::Array(i2)) => self.unify(i1, i2),
            (Type::Result(o1, er1), Type::Result(o2, er2)) => { let s1 = self.unify(o1, o2)?; let s2 = self.unify(&apply_subst_type(&s1, er1), &apply_subst_type(&s1, er2))?; Ok(compose_subst(&s1, &s2)) }
            (Type::Ref(t1), Type::Ref(t2)) | (Type::Linear(t1), Type::Linear(t2)) | (Type::Borrow(t1), Type::Borrow(t2)) => self.unify(t1, t2),
            _ => Err(format!("Mismatch: {:?} vs {:?}", t1, t2)),
        }
    }
}

pub fn apply_subst_type(subst: &Subst, typ: &Type) -> Type {
    match typ {
        Type::Var(n) => subst.get(n).cloned().unwrap_or(typ.clone()),
        Type::Arrow(p, r, e) => Type::Arrow(p.iter().map(|t| apply_subst_type(subst, t)).collect(), Box::new(apply_subst_type(subst, r)), Box::new(apply_subst_type(subst, e))),
        Type::UserDefined(n, a) => Type::UserDefined(n.clone(), a.iter().map(|t| apply_subst_type(subst, t)).collect()),
        Type::Result(o, e) => Type::Result(Box::new(apply_subst_type(subst, o)), Box::new(apply_subst_type(subst, e))),
        Type::Ref(i) => Type::Ref(Box::new(apply_subst_type(subst, i))),
        Type::Linear(i) => Type::Linear(Box::new(apply_subst_type(subst, i))),
        Type::Borrow(i) => Type::Borrow(Box::new(apply_subst_type(subst, i))),
        Type::List(i) => Type::List(Box::new(apply_subst_type(subst, i))),
        Type::Array(i) => Type::Array(Box::new(apply_subst_type(subst, i))),
        Type::Row(es, t) => Type::Row(es.iter().map(|x| apply_subst_type(subst, x)).collect(), t.as_ref().map(|x| Box::new(apply_subst_type(subst, x)))),
        Type::Record(fs) => Type::Record(fs.iter().map(|(n, t)| (n.clone(), apply_subst_type(subst, t))).collect()),
        _ => typ.clone(),
    }
}

fn compose_subst(s1: &Subst, s2: &Subst) -> Subst {
    let mut res = s2.clone(); for (k, v) in s1 { res.insert(k.clone(), apply_subst_type(s2, v)); } res
}

fn get_free_vars_type(typ: &Type) -> HashSet<String> {
    match typ {
        Type::Var(n) => { let mut s = HashSet::new(); s.insert(n.clone()); s }
        Type::Arrow(p, r, e) => { let mut s = get_free_vars_type(r); for t in p { s.extend(get_free_vars_type(t)); } s.extend(get_free_vars_type(e)); s }
        Type::UserDefined(_, a) => { let mut s = HashSet::new(); for t in a { s.extend(get_free_vars_type(t)); } s }
        Type::Result(o, e) => { let mut s = get_free_vars_type(o); s.extend(get_free_vars_type(e)); s }
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::List(i) | Type::Array(i) => get_free_vars_type(i),
        Type::Row(es, t) => { let mut s = HashSet::new(); for e in es { s.extend(get_free_vars_type(e)); } if let Some(x) = t { s.extend(get_free_vars_type(x)); } s }
        Type::Record(fs) => { let mut s = HashSet::new(); for (_, t) in fs { s.extend(get_free_vars_type(t)); } s }
        _ => HashSet::new(),
    }
}

fn get_free_vars_env(env: &TypeEnv) -> HashSet<String> {
    let mut s = HashSet::new(); for sch in env.vars.values() { let ft = get_free_vars_type(&sch.typ); let b: HashSet<_> = sch.vars.iter().cloned().collect(); s.extend(ft.difference(&b).cloned()); } s
}

fn occurs_check(n: &str, t: &Type) -> bool {
    match t {
        Type::Var(m) => n == m,
        Type::Arrow(p, r, e) => occurs_check(n, r) || p.iter().any(|x| occurs_check(n, x)) || occurs_check(n, e),
        Type::UserDefined(_, a) => a.iter().any(|x| occurs_check(n, x)),
        Type::Result(o, e) => occurs_check(n, o) || occurs_check(n, e),
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::List(i) | Type::Array(i) => occurs_check(n, i),
        Type::Row(es, t) => es.iter().any(|x| occurs_check(n, x)) || t.as_ref().map_or(false, |x| occurs_check(n, x)),
        Type::Record(fs) => fs.iter().any(|(_, t)| occurs_check(n, t)),
        _ => false,
    }
}

fn contains_ref(t: &Type) -> bool {
    match t {
        Type::Ref(_) => true,
        Type::Arrow(p, r, e) => contains_ref(r) || p.iter().any(contains_ref) || contains_ref(e),
        Type::UserDefined(_, a) => a.iter().any(contains_ref),
        Type::Result(o, e) => contains_ref(o) || contains_ref(e),
        Type::Linear(i) | Type::Borrow(i) | Type::List(i) | Type::Array(i) => contains_ref(i),
        Type::Row(es, t) => es.iter().any(contains_ref) || t.as_ref().map_or(false, |x| contains_ref(x)),
        Type::Record(fs) => fs.iter().any(|(_, t)| contains_ref(t)),
        _ => false,
    }
}

fn contains_linear(t: &Type) -> bool {
    match t {
        Type::Linear(_) | Type::Array(_) => true,
        Type::Ref(i) | Type::Borrow(i) | Type::List(i) => contains_linear(i),
        Type::Arrow(p, r, e) => contains_linear(r) || p.iter().any(contains_linear) || contains_linear(e),
        Type::UserDefined(_, a) => a.iter().any(contains_linear),
        Type::Result(o, e) => contains_linear(o) || contains_linear(e),
        Type::Row(es, t) => es.iter().any(contains_linear) || t.as_ref().map_or(false, |x| contains_linear(x)),
        Type::Record(fs) => fs.iter().any(|(_, t)| contains_linear(t)),
        _ => false,
    }
}

#[derive(Clone)]
enum PatRef<'a> { Original(&'a Spanned<Pattern>), Synthetic(Pattern, Span) }
impl<'a> PatRef<'a> {
    fn node(&self) -> &Pattern { match self { PatRef::Original(p) => &p.node, PatRef::Synthetic(p, _) => p } }
    fn span(&self) -> Span { match self { PatRef::Original(p) => p.span.clone(), PatRef::Synthetic(_, s) => s.clone() } }
}
