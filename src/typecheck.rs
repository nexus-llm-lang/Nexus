use crate::ast::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct Scheme {
    pub vars: Vec<String>,
    pub typ: Type,
}

#[derive(Debug, Clone)]
pub struct TypeEnv {
    pub vars: HashMap<String, Scheme>,
    pub types: HashMap<String, TypeDef>,
}

type Subst = HashMap<String, Type>;

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            vars: HashMap::new(),
            types: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: String, scheme: Scheme) {
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
}

pub struct TypeChecker {
    pub supply: usize,
    pub env: TypeEnv,
}

impl TypeChecker {
    pub fn new() -> Self {
        let mut env = TypeEnv::new();
        // Mocks for std lib
        let scheme_tx = Scheme {
            vars: vec![],
            typ: Type::UserDefined("Tx".to_string(), vec![]),
        };

        // db_driver
        env.insert(
            "db_driver.begin_tx".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![], Box::new(scheme_tx.typ.clone())),
            },
        );
        env.insert(
            "db_driver.commit".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(
                    vec![Type::UserDefined("Tx".to_string(), vec![])],
                    Box::new(Type::Unit),
                ),
            },
        );
        env.insert(
            "db_driver.rollback".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(
                    vec![Type::UserDefined("Tx".to_string(), vec![])],
                    Box::new(Type::Unit),
                ),
            },
        );

        // log
        env.insert(
            "log.info".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::Str], Box::new(Type::Unit)),
            },
        );

        // stdlib printf (str, i64) -> unit
        env.insert(
            "printf".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::Str, Type::I64], Box::new(Type::Unit)),
            },
        );

        env.insert(
            "print_str".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::Str], Box::new(Type::Unit)),
            },
        );

        env.insert(
            "print_i64".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::I64], Box::new(Type::Unit)),
            },
        );

        TypeChecker { supply: 0, env }
    }

    fn new_var(&mut self) -> Type {
        let n = self.supply;
        self.supply += 1;
        Type::Var(format!("?{}", n))
    }

    pub fn check_program(&mut self, program: &Program) -> Result<(), String> {
        for def in &program.definitions {
            match def {
                TopLevel::TypeDef(td) => {
                    self.env.types.insert(td.name.clone(), td.clone());
                }
                TopLevel::Function(func) => {
                    let scheme = self.generalize_top_level(func);
                    self.env.insert(func.name.clone(), scheme);
                }
                TopLevel::Port(port) => {
                    for sig in &port.functions {
                        let name = format!("{}.{}", port.name, sig.name);
                        let param_types: Vec<Type> =
                            sig.params.iter().map(|p| p.typ.clone()).collect();
                        let typ = Type::Arrow(param_types, Box::new(sig.ret_type.clone()));
                        self.env.insert(name, Scheme { vars: vec![], typ });
                    }
                }
                _ => {}
            }
        }

        for def in &program.definitions {
            if let TopLevel::Function(func) = def {
                if func.name == "main" && func.is_public {
                    return Err("main function must be private (remove 'pub')".to_string());
                }
                self.check_function(func)?;
            }
        }

        // Verify main function signature if it exists
        if let Some(scheme) = self.env.get("main") {
            if !scheme.vars.is_empty() {
                 return Err("main function cannot be generic".to_string());
            }
            if let Type::Arrow(params, ret) = &scheme.typ {
                if !params.is_empty() {
                    return Err("main function cannot take arguments".to_string());
                }
                if **ret != Type::Unit {
                    return Err("main function must return unit".to_string());
                }
            } else {
                return Err("main must be a function".to_string());
            }
        }

        Ok(())
    }

    fn generalize_top_level(&self, func: &Function) -> Scheme {
        let vars: HashSet<String> = func.type_params.iter().cloned().collect();

        let mut param_types = Vec::new();
        for p in &func.params {
            param_types.push(self.convert_user_defined_to_var(&p.typ, &vars));
        }
        let ret_type = self.convert_user_defined_to_var(&func.ret_type, &vars);

        let typ = Type::Arrow(param_types, Box::new(ret_type));

        Scheme {
            vars: func.type_params.clone(),
            typ,
        }
    }

    fn convert_user_defined_to_var(&self, typ: &Type, vars: &HashSet<String>) -> Type {
        match typ {
            Type::UserDefined(n, args) => {
                if args.is_empty() && vars.contains(n) {
                    Type::Var(n.clone())
                } else {
                    Type::UserDefined(
                        n.clone(),
                        args.iter()
                            .map(|a| self.convert_user_defined_to_var(a, vars))
                            .collect(),
                    )
                }
            }
            Type::Result(ok, err) => Type::Result(
                Box::new(self.convert_user_defined_to_var(ok, vars)),
                Box::new(self.convert_user_defined_to_var(err, vars)),
            ),
            Type::Arrow(params, ret) => Type::Arrow(
                params
                    .iter()
                    .map(|p| self.convert_user_defined_to_var(p, vars))
                    .collect(),
                Box::new(self.convert_user_defined_to_var(ret, vars)),
            ),
            Type::Ref(inner) => Type::Ref(Box::new(self.convert_user_defined_to_var(inner, vars))),
            _ => typ.clone(),
        }
    }

    fn check_function(&mut self, func: &Function) -> Result<(), String> {
        let mut local_env = self.env.clone();
        for param in &func.params {
            let typ = param.typ.clone();
            local_env.insert(
                param.sigil.get_key(&param.name),
                Scheme { vars: vec![], typ },
            );
        }
        let expected_ret = func.ret_type.clone();

        // Gravity Rule: Return type cannot contain Ref
        if contains_ref(&expected_ret) {
            return Err(format!(
                "Function {} cannot return a Reference type: {:?}",
                func.name, expected_ret
            ));
        }

        self.infer_body(&func.body, &mut local_env, &expected_ret)
    }

    fn infer_body(
        &mut self,
        body: &[Stmt],
        env: &mut TypeEnv,
        expected_ret: &Type,
    ) -> Result<(), String> {
        for stmt in body {
            match stmt {
                                Stmt::Let {
                                    name, value, sigil, ..
                                } => {
                                    let (s1, t1) = self.infer(env, value, expected_ret)?;
                                    env.apply(&s1);
                
                                    let final_type = if let Sigil::Mutable = sigil {
                                        Type::Ref(Box::new(t1))
                                    } else {
                                        // Gravity Rule: Immutable variables cannot hold References (if they were explicitly Refs?)
                                        // Since expr cannot be Expr::Ref anymore, t1 is the value type.
                                        // If t1 is ALREADY a Ref type (e.g. returned from function?), we should check.
                                        if contains_ref(&t1) {
                                             return Err(format!("Immutable variable {} cannot hold a Reference type: {:?}", name, t1));
                                        }
                                        t1
                                    };
                
                                    let scheme = self.generalize(env, final_type);
                                    let key = sigil.get_key(name);
                
                                    env.insert(key, scheme);
                                }
                
                Stmt::Return(expr) => {
                    let (s1, t1) = self.infer(env, expr, expected_ret)?;
                    env.apply(&s1);
                    let current_ret = apply_subst_type(&s1, expected_ret);
                    let s2 = self.unify(&t1, &current_ret)?;
                    env.apply(&s2);
                }
                Stmt::Expr(expr) => {
                    let (s1, _) = self.infer(env, expr, expected_ret)?;
                    env.apply(&s1);
                }

                Stmt::Assign { name, value, sigil } => {
                    // Gravity Rule: Mutation requires Mutable sigil
                    if let Sigil::Immutable = sigil {
                        return Err(format!(
                            "Cannot use immutable variable {} for mutation. Use ~{} instead.",
                            name, name
                        ));
                    }

                    let (s_val, t_val) = self.infer(env, value, expected_ret)?;
                    env.apply(&s_val);

                    let key = sigil.get_key(name);
                    if let Some(scheme) = env.get(&key) {
                        let t_var = self.instantiate(scheme);
                        if let Type::Ref(inner) = t_var {
                            let s_assign = self.unify(&t_val, &inner)?;
                            env.apply(&s_assign);
                        } else {
                            return Err(format!("Cannot assign to non-ref variable {}", key));
                        }
                    } else {
                        return Err(format!("Variable not found: {}", key));
                    }
                }
                Stmt::Conc(tasks) => {
                    for task in tasks {
                        self.check_task(task, env)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn check_task(&mut self, task: &Function, outer_env: &TypeEnv) -> Result<(), String> {
        // Gravity Rule: Parallel tasks cannot capture Mutable variables (~ vars)
        let mut task_env = TypeEnv::new();
        task_env.types = outer_env.types.clone();

        for (key, scheme) in &outer_env.vars {
            if !key.starts_with('~') {
                task_env.insert(key.clone(), scheme.clone());
            }
        }

        // Tasks return Unit
        self.infer_body(&task.body, &mut task_env, &Type::Unit)
    }

    pub fn check_repl_stmt(&mut self, stmt: &Stmt) -> Result<Type, String> {
        let mut env = std::mem::replace(&mut self.env, TypeEnv::new());
        let res = (|| {
            match stmt {
                Stmt::Expr(expr) => {
                    let (s, t) = self.infer(&env, expr, &Type::Unit)?;
                    env.apply(&s);
                    Ok(t)
                }
                _ => {
                    self.infer_body(&[stmt.clone()], &mut env, &Type::Unit)?;
                    Ok(Type::Unit)
                }
            }
        })();
        self.env = env;
        res
    }

    fn infer(
        &mut self,
        env: &TypeEnv,
        expr: &Expr,
        expected_ret: &Type,
    ) -> Result<(Subst, Type), String> {
        match expr {
            Expr::Literal(lit) => {
                let t = match lit {
                    Literal::Int(_) => Type::I64,
                    Literal::Bool(_) => Type::Bool,
                    Literal::String(_) => Type::Str,
                    Literal::Unit => Type::Unit,
                };
                Ok((HashMap::new(), t))
            }
            Expr::Variable(name, sigil) => {
                let key = sigil.get_key(name);

                if let Some(scheme) = env.get(&key) {
                    let mut t = self.instantiate(scheme);
                    if let Sigil::Mutable = sigil {
                        if let Type::Ref(inner) = t {
                            t = *inner;
                        }
                    }
                    Ok((HashMap::new(), t))
                } else {
                    Err(format!("Variable not found: {}", key))
                }
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                let (s1, t1) = self.infer(env, lhs, expected_ret)?;
                let (s2, t2) = self.infer(env, rhs, expected_ret)?;
                let mut s = compose_subst(&s1, &s2);
                match op.as_str() {
                    "+" | "-" | "*" | "/" => {
                        let s3 = self.unify(&apply_subst_type(&s, &t1), &Type::I64)?;
                        s = compose_subst(&s, &s3);
                        let s4 = self.unify(&apply_subst_type(&s, &t2), &Type::I64)?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::I64))
                    }
                    "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                        let s3 = self.unify(&apply_subst_type(&s, &t1), &Type::I64)?;
                        s = compose_subst(&s, &s3);
                        let s4 = self.unify(&apply_subst_type(&s, &t2), &Type::I64)?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Bool))
                    }
                    _ => Err(format!("Unknown operator {}", op)),
                }
            }
            Expr::Call { func, args, .. } => {
                let (mut subst, func_type) = if let Some(scheme) = env.get(func) {
                    (HashMap::new(), self.instantiate(scheme))
                } else {
                    return Err(format!("Function not found: {}", func));
                };

                let ret_type = self.new_var();
                let param_types: Vec<Type> = args.iter().map(|_| self.new_var()).collect();
                let call_type = Type::Arrow(param_types.clone(), Box::new(ret_type.clone()));

                let s_fn = self.unify(&func_type, &call_type)?;
                subst = compose_subst(&subst, &s_fn);

                for (param_type, (_, arg_expr)) in param_types.iter().zip(args) {
                    let (s_arg, t_arg) = self.infer(env, arg_expr, expected_ret)?;
                    subst = compose_subst(&subst, &s_arg);

                    let current_param = apply_subst_type(&subst, param_type);
                    let s_unify = self.unify(&t_arg, &current_param)?;
                    subst = compose_subst(&subst, &s_unify);
                }

                Ok((subst.clone(), apply_subst_type(&subst, &ret_type)))
            }
            Expr::Constructor(name, args) => {
                if name == "Ok" && args.len() == 1 {
                    let (s, t) = self.infer(env, &args[0], expected_ret)?;
                    return Ok((s, Type::Result(Box::new(t), Box::new(self.new_var()))));
                }
                if name == "Err" && args.len() == 1 {
                    let (s, t) = self.infer(env, &args[0], expected_ret)?;
                    return Ok((s, Type::Result(Box::new(self.new_var()), Box::new(t))));
                }
                Ok((HashMap::new(), Type::Unit))
            }
            Expr::Record(fields) => {
                let mut subst = HashMap::new();
                for (_, expr) in fields {
                    let (s, _) = self.infer(env, expr, expected_ret)?;
                    subst = compose_subst(&subst, &s);
                }
                Ok((
                    subst,
                    Type::UserDefined("AnonymousRecord".to_string(), vec![]),
                ))
            }
            Expr::FieldAccess(receiver, field_name) => {
                let (s1, t_rec) = self.infer(env, receiver, expected_ret)?;
                if let Type::UserDefined(type_name, type_args) = &t_rec {
                    if let Some(td) = env.types.get(type_name) {
                        if let Some((_, f_type)) = td.fields.iter().find(|(n, _)| n == field_name) {
                            let mut subst = HashMap::new();
                            for (p, a) in td.type_params.iter().zip(type_args) {
                                subst.insert(p.clone(), a.clone());
                            }
                            return Ok((s1, apply_subst_type(&subst, f_type)));
                        }
                    }
                }
                Err(format!(
                    "Field {} not found on type {:?}",
                    field_name, t_rec
                ))
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let (s1, t_cond) = self.infer(env, cond, expected_ret)?;
                let s = compose_subst(&s1, &self.unify(&t_cond, &Type::Bool)?);
                let mut local_env = env.clone();
                local_env.apply(&s);
                self.infer_body(then_branch, &mut local_env, expected_ret)?;
                if let Some(else_branch) = else_branch {
                    self.infer_body(else_branch, &mut local_env, expected_ret)?;
                }
                Ok((s, Type::Unit))
            }
            Expr::Match { target, cases } => {
                let (s1, t_target) = self.infer(env, target, expected_ret)?;
                let mut s = s1;
                for case in cases {
                    let mut local_env = env.clone();
                    local_env.apply(&s);
                    let s_match = self.bind_pattern(
                        &case.pattern,
                        &apply_subst_type(&s, &t_target),
                        &mut local_env,
                    )?;
                    s = compose_subst(&s, &s_match);
                    local_env.apply(&s_match);
                    self.infer_body(&case.body, &mut local_env, expected_ret)?;
                }
                Ok((s, Type::Unit))
            }

        }
    }

    fn bind_pattern(
        &mut self,
        pattern: &Pattern,
        t_target: &Type,
        env: &mut TypeEnv,
    ) -> Result<Subst, String> {
        match pattern {
            Pattern::Variable(name, _) => {
                env.insert(
                    name.clone(),
                    Scheme {
                        vars: vec![],
                        typ: t_target.clone(),
                    },
                );
                Ok(HashMap::new())
            }
            Pattern::Constructor(name, pats) => {
                if name == "Ok" && pats.len() == 1 {
                    let t_ok = self.new_var();
                    let t_err = self.new_var();
                    let s = self.unify(
                        t_target,
                        &Type::Result(Box::new(t_ok.clone()), Box::new(t_err)),
                    )?;
                    let s_pat = self.bind_pattern(&pats[0], &apply_subst_type(&s, &t_ok), env)?;
                    return Ok(compose_subst(&s, &s_pat));
                }
                if name == "Err" && pats.len() == 1 {
                    let t_ok = self.new_var();
                    let t_err = self.new_var();
                    let s = self.unify(
                        t_target,
                        &Type::Result(Box::new(t_ok), Box::new(t_err.clone())),
                    )?;
                    let s_pat = self.bind_pattern(&pats[0], &apply_subst_type(&s, &t_err), env)?;
                    return Ok(compose_subst(&s, &s_pat));
                }
                Err(format!("Unknown constructor in pattern: {}", name))
            }
            Pattern::Literal(lit) => {
                let t_lit = match lit {
                    Literal::Int(_) => Type::I64,
                    Literal::Bool(_) => Type::Bool,
                    Literal::String(_) => Type::Str,
                    Literal::Unit => Type::Unit,
                };
                self.unify(t_target, &t_lit)
            }
            Pattern::Wildcard => Ok(HashMap::new()),
        }
    }

    fn instantiate(&mut self, scheme: &Scheme) -> Type {
        let mut subst = HashMap::new();
        for var in &scheme.vars {
            subst.insert(var.clone(), self.new_var());
        }
        apply_subst_type(&subst, &scheme.typ)
    }

    fn generalize(&self, env: &TypeEnv, typ: Type) -> Scheme {
        let env_vars = get_free_vars_env(env);
        let type_vars = get_free_vars_type(&typ);
        let free: Vec<String> = type_vars.difference(&env_vars).cloned().collect();
        Scheme { vars: free, typ }
    }

    fn unify(&mut self, t1: &Type, t2: &Type) -> Result<Subst, String> {
        match (t1, t2) {
            (t1, t2) if t1 == t2 => Ok(HashMap::new()),
            (Type::Var(n), t) | (t, Type::Var(n)) => {
                if occurs_check(n, t) {
                    return Err("Recursive type".into());
                }
                let mut s = HashMap::new();
                s.insert(n.clone(), t.clone());
                Ok(s)
            }
            (Type::Arrow(p1, r1), Type::Arrow(p2, r2)) => {
                if p1.len() != p2.len() {
                    return Err("Arity mismatch".into());
                }
                let mut s = HashMap::new();
                for (a, b) in p1.iter().zip(p2) {
                    let s_new = self.unify(&apply_subst_type(&s, a), &apply_subst_type(&s, b))?;
                    s = compose_subst(&s, &s_new);
                }
                let s_ret = self.unify(&apply_subst_type(&s, r1), &apply_subst_type(&s, r2))?;
                Ok(compose_subst(&s, &s_ret))
            }
            (Type::UserDefined(n1, args1), Type::UserDefined(n2, args2)) if n1 == n2 => {
                if args1.len() != args2.len() {
                    return Err("Generic arity mismatch".into());
                }
                let mut s = HashMap::new();
                for (a, b) in args1.iter().zip(args2) {
                    let s_new = self.unify(&apply_subst_type(&s, a), &apply_subst_type(&s, b))?;
                    s = compose_subst(&s, &s_new);
                }
                Ok(s)
            }
            (Type::Result(ok1, err1), Type::Result(ok2, err2)) => {
                let s1 = self.unify(ok1, ok2)?;
                let s2 = self.unify(&apply_subst_type(&s1, err1), &apply_subst_type(&s1, err2))?;
                Ok(compose_subst(&s1, &s2))
            }
            (Type::Ref(t1), Type::Ref(t2)) => self.unify(t1, t2),
            _ => Err(format!("Type mismatch: {:?} vs {:?}", t1, t2)),
        }
    }
}

pub fn apply_subst_type(subst: &Subst, typ: &Type) -> Type {
    match typ {
        Type::Var(n) => subst.get(n).cloned().unwrap_or(typ.clone()),
        Type::Arrow(params, ret) => Type::Arrow(
            params.iter().map(|p| apply_subst_type(subst, p)).collect(),
            Box::new(apply_subst_type(subst, ret)),
        ),
        Type::UserDefined(n, args) => Type::UserDefined(
            n.clone(),
            args.iter().map(|a| apply_subst_type(subst, a)).collect(),
        ),
        Type::Result(ok, err) => Type::Result(
            Box::new(apply_subst_type(subst, ok)),
            Box::new(apply_subst_type(subst, err)),
        ),
        Type::Ref(inner) => Type::Ref(Box::new(apply_subst_type(subst, inner))),
        _ => typ.clone(),
    }
}

fn compose_subst(s1: &Subst, s2: &Subst) -> Subst {
    let mut result = s2.clone();
    for (k, v) in s1 {
        result.insert(k.clone(), apply_subst_type(s2, v));
    }
    result
}

fn get_free_vars_type(typ: &Type) -> HashSet<String> {
    match typ {
        Type::Var(n) => {
            let mut s = HashSet::new();
            s.insert(n.clone());
            s
        }
        Type::Arrow(params, ret) => {
            let mut s = get_free_vars_type(ret);
            for p in params {
                s.extend(get_free_vars_type(p));
            }
            s
        }
        Type::UserDefined(_, args) => {
            let mut s = HashSet::new();
            for a in args {
                s.extend(get_free_vars_type(a));
            }
            s
        }
        Type::Result(ok, err) => {
            let mut s = get_free_vars_type(ok);
            s.extend(get_free_vars_type(err));
            s
        }
        Type::Ref(inner) => get_free_vars_type(inner),
        _ => HashSet::new(),
    }
}

fn get_free_vars_env(env: &TypeEnv) -> HashSet<String> {
    let mut s = HashSet::new();
    for scheme in env.vars.values() {
        let free_in_typ = get_free_vars_type(&scheme.typ);
        let bound: HashSet<String> = scheme.vars.iter().cloned().collect();
        s.extend(free_in_typ.difference(&bound).cloned());
    }
    s
}

fn occurs_check(name: &str, typ: &Type) -> bool {
    match typ {
        Type::Var(n) => n == name,
        Type::Arrow(params, ret) => {
            occurs_check(name, ret) || params.iter().any(|p| occurs_check(name, p))
        }
        Type::UserDefined(_, args) => args.iter().any(|a| occurs_check(name, a)),
        Type::Result(ok, err) => occurs_check(name, ok) || occurs_check(name, err),
        Type::Ref(inner) => occurs_check(name, inner),
        _ => false,
    }
}

fn contains_ref(typ: &Type) -> bool {
    match typ {
        Type::Ref(_) => true,
        Type::Arrow(params, ret) => contains_ref(ret) || params.iter().any(contains_ref),
        Type::UserDefined(_, args) => args.iter().any(contains_ref),
        Type::Result(ok, err) => contains_ref(ok) || contains_ref(err),
        _ => false,
    }
}
