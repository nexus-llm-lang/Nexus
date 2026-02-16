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
    pub linear_vars: HashSet<String>,
}

type Subst = HashMap<String, Type>;

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            vars: HashMap::new(),
            types: HashMap::new(),
            linear_vars: HashSet::new(),
        }
    }

    pub fn insert(&mut self, name: String, scheme: Scheme) {
        if let Type::Linear(_) = scheme.typ {
            self.linear_vars.insert(name.clone());
        }
        self.vars.insert(name, scheme);
    }

    pub fn get(&self, name: &str) -> Option<&Scheme> {
        self.vars.get(name)
    }
    
    pub fn consume(&mut self, name: &str) -> Result<(), String> {
        if self.linear_vars.remove(name) {
            Ok(())
        } else {
            // It might be double use, or it might be not linear.
            // Caller ensures it IS linear type.
            Err(format!("Linear variable '{}' used more than once", name))
        }
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
                typ: Type::Arrow(vec![], Box::new(scheme_tx.typ.clone()), Box::new(Type::Row(vec![], None))),
            },
        );
        env.insert(
            "db_driver.commit".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(
                    vec![Type::UserDefined("Tx".to_string(), vec![])],
                    Box::new(Type::Unit),
                    Box::new(Type::Row(vec![], None)),
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
                    Box::new(Type::Row(vec![], None)),
                ),
            },
        );

        // log
        env.insert(
            "log.info".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::Str], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))),
            },
        );

        // stdlib printf (str, i64) -> unit
        env.insert(
            "printf".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::Str, Type::I64], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))),
            },
        );

        env.insert(
            "print_str".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::Str], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))),
            },
        );

        env.insert(
            "print_i64".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::I64], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))),
            },
        );

        env.insert(
            "drop_i64".to_string(),
            Scheme {
                vars: vec![],
                typ: Type::Arrow(vec![Type::Linear(Box::new(Type::I64))], Box::new(Type::Unit), Box::new(Type::Row(vec![], None))),
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
                TopLevel::Port(port) => {
                    for sig in &port.functions {
                        let name = format!("{}.{}", port.name, sig.name);
                        let param_types: Vec<Type> =
                            sig.params.iter().map(|p| p.typ.clone()).collect();
                        let typ = Type::Arrow(
                            param_types,
                            Box::new(sig.ret_type.clone()),
                            Box::new(sig.effects.clone()),
                        );
                        self.env.insert(name, Scheme { vars: vec![], typ });
                    }
                }
                TopLevel::Function(func) => {
                    let scheme = self.generalize_top_level(func);
                    self.env.insert(func.name.clone(), scheme);
                }
                _ => {}
            }
        }

        for def in &program.definitions {
            match def {
                TopLevel::Function(func) => {
                    if func.name == "main" && func.is_public {
                        return Err("main function must be private (remove 'pub')".to_string());
                    }
                    self.check_function(func)?;
                }
                TopLevel::Handler(handler) => {
                    self.check_handler(handler)?;
                }
                _ => {}
            }
        }

        // Verify main function signature if it exists
        if let Some(scheme) = self.env.get("main") {
            if !scheme.vars.is_empty() {
                 return Err("main function cannot be generic".to_string());
            }
            if let Type::Arrow(params, ret, _) = &scheme.typ {
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
        
        let effects = self.convert_user_defined_to_var(&func.effects, &vars);

        let typ = Type::Arrow(param_types, Box::new(ret_type), Box::new(effects));

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
            Type::Arrow(params, ret, effects) => Type::Arrow(
                params
                    .iter()
                    .map(|p| self.convert_user_defined_to_var(p, vars))
                    .collect(),
                Box::new(self.convert_user_defined_to_var(ret, vars)),
                Box::new(self.convert_user_defined_to_var(effects, vars)),
            ),
            Type::Ref(inner) => Type::Ref(Box::new(self.convert_user_defined_to_var(inner, vars))),
            Type::Linear(inner) => {
                Type::Linear(Box::new(self.convert_user_defined_to_var(inner, vars)))
            }
            Type::Row(effs, tail) => Type::Row(
                effs.iter()
                    .map(|e| self.convert_user_defined_to_var(e, vars))
                    .collect(),
                tail.as_ref()
                    .map(|t| Box::new(self.convert_user_defined_to_var(t, vars))),
            ),
            Type::Record(fields) => Type::Record(
                fields
                    .iter()
                    .map(|(n, t)| (n.clone(), self.convert_user_defined_to_var(t, vars)))
                    .collect(),
            ),
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
        let expected_eff = func.effects.clone();

        // Gravity Rule: Return type cannot contain Ref
        if contains_ref(&expected_ret) {
            return Err(format!(
                "Function {} cannot return a Reference type: {:?}",
                func.name, expected_ret
            ));
        }

        eprintln!("DEBUG: check_function start for {}. linear_vars: {:?}", func.name, local_env.linear_vars);
        self.infer_body(&func.body, &mut local_env, &expected_ret, &expected_eff)?;
        eprintln!("DEBUG: check_function end for {}. linear_vars: {:?}", func.name, local_env.linear_vars);
        
        if !local_env.linear_vars.is_empty() {
             return Err(format!("Unused linear variables at end of function {}: {:?}", func.name, local_env.linear_vars));
        }
        Ok(())
    }

    fn check_handler(&mut self, handler: &Handler) -> Result<(), String> {
        for func in &handler.functions {
            let full_name = format!("{}.{}", handler.port_name, func.name);
            if let Some(scheme) = self.env.get(&full_name).cloned() {
                // Ensure the function signature in handler matches the port's signature.
                let func_type = self.generalize_top_level(func).typ;
                self.unify(&scheme.typ, &func_type)?;

                // Now check the body
                self.check_function(func)?;
            } else {
                return Err(format!(
                    "Function {} not found in port {}",
                    func.name, handler.port_name
                ));
            }
        }
        Ok(())
    }

    fn infer_body(
        &mut self,
        body: &[Stmt],
        env: &mut TypeEnv,
        expected_ret: &Type,
        expected_eff: &Type,
    ) -> Result<(), String> {
        for stmt in body {
            match stmt {
                                Stmt::Let {
                                    name, value, sigil, ..
                                } => {
                                    let (s1, t1) = self.infer(env, value, expected_ret, expected_eff)?;
                                    env.apply(&s1);
                
                                    let final_type = match sigil {
                                        Sigil::Mutable => {
                                            if contains_linear(&t1) {
                                                return Err(format!("Cannot create mutable reference to linear type: {:?}", t1));
                                            }
                                            Type::Ref(Box::new(t1))
                                        },
                                        Sigil::Linear => Type::Linear(Box::new(t1)),
                                        Sigil::Immutable => {
                                            // Gravity Rule: Immutable variables cannot hold References
                                            if contains_ref(&t1) {
                                                 return Err(format!("Immutable variable {} cannot hold a Reference type: {:?}", name, t1));
                                            }
                                            t1
                                        }
                                    };
                
                                    let scheme = self.generalize(env, final_type);
                                    let key = sigil.get_key(name);
                
                                    env.insert(key.clone(), scheme);
                                    // if name == "_" {
                                    //    env.linear_vars.remove(&key);
                                    // }
                                    eprintln!("DEBUG: Stmt::Let {}. linear_vars: {:?}", key, env.linear_vars);
                                }
                
                Stmt::Return(expr) => {
                    let (s1, t1) = self.infer(env, expr, expected_ret, expected_eff)?;
                    env.apply(&s1);
                    
                    if !env.linear_vars.is_empty() {
                         return Err(format!("Unused linear variables at return: {:?}", env.linear_vars));
                    }

                    let current_ret = apply_subst_type(&s1, expected_ret);
                    let s2 = self.unify(&t1, &current_ret)?;
                    env.apply(&s2);
                }
                Stmt::Expr(expr) => {
                    let (s1, _) = self.infer(env, expr, expected_ret, expected_eff)?;
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

                    let (s_val, t_val) = self.infer(env, value, expected_ret, expected_eff)?;
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
                Stmt::Try { body, catch_param, catch_body } => {
                    let mut env_try = env.clone();
                    self.infer_body(body, &mut env_try, expected_ret, expected_eff)?;
                    
                    let mut env_catch = env.clone();
                    env_catch.insert(catch_param.clone(), Scheme { vars: vec![], typ: Type::Str });
                    self.infer_body(catch_body, &mut env_catch, expected_ret, expected_eff)?;
                    
                    // Merge linear vars
                    let avail_try = &env_try.linear_vars;
                    let avail_catch = &env_catch.linear_vars;
                    if avail_try != avail_catch {
                        return Err("Linear variable usage mismatch in try/catch".to_string());
                    }
                    env.linear_vars = env_try.linear_vars;
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
        self.infer_body(&task.body, &mut task_env, &Type::Unit, &Type::Unit)
    }

        pub fn check_repl_stmt(&mut self, stmt: &Stmt) -> Result<Type, String> {

            let mut env = std::mem::replace(&mut self.env, TypeEnv::new());

            let res = (|| {

                match stmt {

                    Stmt::Expr(expr) => {

                        let eff = self.new_var();

                        let (s, t) = self.infer(&mut env, expr, &Type::Unit, &eff)?;

                        env.apply(&s);

                        Ok(t)

                    }

                    _ => {

                        let eff = self.new_var();

                        self.infer_body(&[stmt.clone()], &mut env, &Type::Unit, &eff)?;

                        Ok(Type::Unit)

                    }

                }

            })();

            self.env = env;

            res

        }

    fn infer(
        &mut self,
        env: &mut TypeEnv,
        expr: &Expr,
        expected_ret: &Type,
        expected_eff: &Type,
    ) -> Result<(Subst, Type), String> {
        eprintln!("DEBUG: infer expr: {:?}", expr);
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
                eprintln!("DEBUG: infer Variable key={}", key);

                // Clone scheme to avoid holding immutable borrow on env while consuming
                if let Some(scheme) = env.get(&key).cloned() {
                    let mut t = self.instantiate(&scheme);
                    if let Sigil::Mutable = sigil {
                        if let Type::Ref(inner) = t {
                            t = *inner;
                        }
                    }
                    // Consume if Type is Linear
                    if let Type::Linear(_) = t {
                        if let Err(e) = env.consume(&key) {
                            return Err(e);
                        }
                    }
                    Ok((HashMap::new(), t))
                } else {
                    Err(format!("Variable not found: {}", key))
                }
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                let (s1, t1) = self.infer(env, lhs, expected_ret, expected_eff)?;
                let (s2, t2) = self.infer(env, rhs, expected_ret, expected_eff)?;
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
                let (mut subst, func_type) = if let Some(scheme) = env.get(func).cloned() {
                    (HashMap::new(), self.instantiate(&scheme))
                } else {
                    return Err(format!("Function not found: {}", func));
                };

                let ret_type = self.new_var();
                let param_types: Vec<Type> = args.iter().map(|_| self.new_var()).collect();
                let eff_call = self.new_var();
                let call_type = Type::Arrow(
                    param_types.clone(),
                    Box::new(ret_type.clone()),
                    Box::new(eff_call.clone()),
                );

                let s_fn = self.unify(&func_type, &call_type)?;
                subst = compose_subst(&subst, &s_fn);
                
                // Check effects
                let s_eff = self.unify(&apply_subst_type(&subst, expected_eff), &apply_subst_type(&subst, &eff_call))?;
                subst = compose_subst(&subst, &s_eff);

                for (param_type, (_, arg_expr)) in param_types.iter().zip(args) {
                    let (s_arg, t_arg) = self.infer(env, arg_expr, expected_ret, expected_eff)?;
                    subst = compose_subst(&subst, &s_arg);

                    let current_param = apply_subst_type(&subst, param_type);
                    let s_unify = self.unify(&t_arg, &current_param)?;
                    subst = compose_subst(&subst, &s_unify);
                }

                Ok((subst.clone(), apply_subst_type(&subst, &ret_type)))
            }
            Expr::Constructor(name, args) => {
                if name == "Ok" && args.len() == 1 {
                    let (s, t) = self.infer(env, &args[0], expected_ret, expected_eff)?;
                    return Ok((s, Type::Result(Box::new(t), Box::new(self.new_var()))));
                }
                if name == "Err" && args.len() == 1 {
                    let (s, t) = self.infer(env, &args[0], expected_ret, expected_eff)?;
                    return Ok((s, Type::Result(Box::new(self.new_var()), Box::new(t))));
                }
                Ok((HashMap::new(), Type::Unit))
            }
            Expr::Record(fields) => {
                let mut subst = HashMap::new();
                let mut record_fields = Vec::new();
                for (name, expr) in fields {
                    let (s, t) = self.infer(env, expr, expected_ret, expected_eff)?;
                    subst = compose_subst(&subst, &s);
                    record_fields.push((name.clone(), t));
                }
                Ok((
                    subst,
                    Type::Record(record_fields),
                ))
            }
            Expr::FieldAccess(receiver, field_name) => {
                let (s1, t_rec) = self.infer(env, receiver, expected_ret, expected_eff)?;
                let t_rec = apply_subst_type(&s1, &t_rec);

                if let Type::Record(fields) = &t_rec {
                    if let Some((_, t)) = fields.iter().find(|(n, _)| n == field_name) {
                        return Ok((s1, t.clone()));
                    }
                    return Err(format!("Field {} not found in record", field_name));
                }

                if let Type::UserDefined(type_name, type_args) = &t_rec {
                    if let Some(td) = env.types.get(type_name).cloned() {
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
                let (s1, t_cond) = self.infer(env, cond, expected_ret, expected_eff)?;
                let s = compose_subst(&s1, &self.unify(&t_cond, &Type::Bool)?);
                
                let mut env_then = env.clone();
                env_then.apply(&s);
                self.infer_body(then_branch, &mut env_then, expected_ret, expected_eff)?;
                
                let mut env_else = env.clone();
                env_else.apply(&s);
                if let Some(else_branch) = else_branch {
                    self.infer_body(else_branch, &mut env_else, expected_ret, expected_eff)?;
                } else {
                     // Implicit else branch (returns Unit). 
                     // But implicit else does nothing, so it consumes NOTHING.
                     // If 'then' branch consumes linear vars, this is a mismatch!
                }
                
                // Merge linear vars
                // Both branches must leave the SAME set of linear vars (Exactly Once).
                // Or rather: "Consumed" set must be equal.
                // env.linear_vars is "Available".
                // Available_then must equal Available_else.
                let available_then = &env_then.linear_vars;
                let available_else = &env_else.linear_vars;
                
                if available_then != available_else {
                    let diff1: Vec<_> = available_then.difference(available_else).collect();
                    let diff2: Vec<_> = available_else.difference(available_then).collect();
                    return Err(format!("Linear variable usage mismatch in branches. Unmatched: {:?}, {:?}", diff1, diff2));
                }
                
                // Update outer env
                env.linear_vars = env_then.linear_vars;
                
                Ok((s, Type::Unit))
            }
            Expr::Match { target, cases } => {
                let (s1, t_target) = self.infer(env, target, expected_ret, expected_eff)?;
                let mut s = s1;
                
                if let Err(e) = self.check_exhaustiveness(&apply_subst_type(&s, &t_target), cases) {
                    return Err(e);
                }
                
                // For match, we need to ensure all cases consume same linear vars.
                // We'll track the "resulting available vars" from the first case, and compare others.
                let mut resulting_linear_vars: Option<HashSet<String>> = None;

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
                    self.infer_body(&case.body, &mut local_env, expected_ret, expected_eff)?;
                    
                    if let Some(prev) = &resulting_linear_vars {
                        if prev != &local_env.linear_vars {
                             return Err("Linear variable usage mismatch in match cases".to_string());
                        }
                    } else {
                        resulting_linear_vars = Some(local_env.linear_vars.clone());
                    }
                }
                
                                if let Some(res_vars) = resulting_linear_vars {
                
                                    env.linear_vars = res_vars;
                
                                }
                
                                
                
                                Ok((s, Type::Unit))
                
                            }
                
                                        Expr::Raise(expr) => {
                
                                            let (s, t) = self.infer(env, expr, expected_ret, expected_eff)?;
                
                                            // Expect string error message
                
                                            let s_str = self.unify(&t, &Type::Str)?;
                
                                            let s = compose_subst(&s, &s_str);
                
                                            
                
                                            // Raise returns any type (diverges)
                
                                            let ret = self.new_var();
                
                                            Ok((s, ret))
                
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
            Pattern::Wildcard => {
                if contains_linear(t_target) {
                    return Err(format!("Cannot discard linear type with wildcard pattern: {:?}", t_target));
                }
                Ok(HashMap::new())
            },
            Pattern::Record(pat_fields, is_open) => {
                // Resolve target to fields
                let target_fields_map = match t_target {
                    Type::Record(fields) => {
                        let mut map = HashMap::new();
                        for (n, t) in fields { map.insert(n.clone(), t.clone()); }
                        map
                    },
                    Type::UserDefined(name, args) => {
                        if let Some(td) = env.types.get(name).cloned() {
                            let mut map = HashMap::new();
                            let mut subst = HashMap::new();
                            for (p, a) in td.type_params.iter().zip(args) {
                                subst.insert(p.clone(), a.clone());
                            }
                            for (n, t) in &td.fields {
                                map.insert(n.clone(), apply_subst_type(&subst, t));
                            }
                            map
                        } else {
                            return Err(format!("Unknown type or not a record: {}", name));
                        }
                    },
                    _ => return Err(format!("Pattern record match against non-record type: {:?}", t_target)),
                };

                let mut subst = HashMap::new();
                let mut matched_fields = HashSet::new();

                for (name, pat) in pat_fields {
                    if let Some(t_field) = target_fields_map.get(name) {
                        let s_field = self.bind_pattern(pat, &apply_subst_type(&subst, t_field), env)?;
                        subst = compose_subst(&subst, &s_field);
                        matched_fields.insert(name.clone());
                    } else {
                         return Err(format!("Field {} not found in target type", name));
                    }
                }

                if !is_open {
                    for key in target_fields_map.keys() {
                        if !matched_fields.contains(key) {
                            return Err(format!("Missing field in pattern: {}", key));
                        }
                    }
                }
                
                Ok(subst)
            }
        }
    }

    fn instantiate(&mut self, scheme: &Scheme) -> Type {
        let mut subst = HashMap::new();
        for var in &scheme.vars {
            subst.insert(var.clone(), self.new_var());
        }
        apply_subst_type(&subst, &scheme.typ)
    }

    fn check_exhaustiveness(&self, target_type: &Type, cases: &[MatchCase]) -> Result<(), String> {
        let patterns: Vec<&Pattern> = cases.iter().map(|c| &c.pattern).collect();
        // Convert to matrix (Nx1)
        let matrix: Vec<Vec<&Pattern>> = patterns.iter().map(|p| vec![*p]).collect();
        let types = vec![target_type];
        self.check_matrix(&matrix, &types)
    }

    fn check_matrix(&self, matrix: &[Vec<&Pattern>], types: &[&Type]) -> Result<(), String> {
        if matrix.is_empty() {
            return Err("Non-exhaustive patterns".to_string());
        }
        if types.is_empty() {
            return Ok(());
        }

        let first_type = types[0];
        let rest_types = &types[1..];

        // Check if first column has wildcard
        // But we need to handle constructors.
        // Simplified: Split based on type.

        match first_type {
            Type::Bool => {
                self.check_constructor_matrix(matrix, "true", 0, &[], rest_types)?;
                self.check_constructor_matrix(matrix, "false", 0, &[], rest_types)?;
                Ok(())
            },
            Type::Result(ok, err) => {
                self.check_constructor_matrix(matrix, "Ok", 1, &[&**ok], rest_types)?;
                self.check_constructor_matrix(matrix, "Err", 1, &[&**err], rest_types)?;
                Ok(())
            },
            Type::Record(fields) => {
                // Product type. Expand.
                // New types: fields types + rest_types.
                let mut field_types: Vec<&Type> = fields.iter().map(|(_, t)| t).collect();
                field_types.extend_from_slice(rest_types);
                
                // Expand matrix
                let mut new_matrix = Vec::new();
                for row in matrix {
                    let p = row[0];
                    let rest_row = &row[1..];
                    
                    match p {
                        Pattern::Record(pat_fields, is_open) => {
                            // Map pat_fields to ordered fields.
                            // If is_open and missing, Wildcard.
                            // If !is_open and missing, impossible (checked by bind).
                            let mut new_row = Vec::new();
                            for (fname, _) in fields {
                                if let Some((_, pat)) = pat_fields.iter().find(|(n, _)| n == fname) {
                                    new_row.push(pat);
                                } else if *is_open {
                                    new_row.push(&Pattern::Wildcard);
                                } else {
                                    // Should not happen if well-typed
                                    new_row.push(&Pattern::Wildcard); 
                                }
                            }
                            new_row.extend_from_slice(rest_row);
                            new_matrix.push(new_row);
                        },
                        Pattern::Wildcard | Pattern::Variable(..) => {
                             // Expand wildcard to wildcards for all fields
                             let mut new_row = vec![&Pattern::Wildcard; fields.len()];
                             new_row.extend_from_slice(rest_row);
                             new_matrix.push(new_row);
                        },
                        _ => {} // Skip mismatched patterns (shouldn't happen)
                    }
                }
                self.check_matrix(&new_matrix, &field_types)
            },
            Type::Unit => {
                self.check_constructor_matrix(matrix, "()", 0, &[], rest_types)?;
                Ok(())
            }
            Type::UserDefined(name, _) => {
                 // Try to resolve to Record/TypeDef?
                 // If nominal type, we need to know if it's enum or struct.
                 // Currently only TypeDef (Record-like).
                 // So treat as Record if found in env?
                 // check_exhaustiveness calls check_matrix.
                 // env is not passed to check_matrix easily (self has env, but check_matrix is distinct?)
                 // self is TypeChecker.
                 if let Some(td) = self.env.types.get(name) {
                     // It is a struct (record).
                     // Treat same as Record.
                     // Construct Type::Record equivalent for logic.
                     // But types slice contains references.
                     // We need to construct owned Type::Record or handle UserDefined same as Record logic.
                     // Duplicate Record logic or recursion?
                     // I'll assume Record logic logic.
                     // We need to resolve generics? Yes.
                     // But types[0] is instantiated type?
                     // No, Type::UserDefined holds args.
                     // So we can resolve field types.
                     // This is complex.
                     // For now, assume UserDefined requires Wildcard unless it is opaque?
                     // Fallback to "Wildcard required".
                     self.check_wildcard_matrix(matrix, rest_types)
                 } else {
                     self.check_wildcard_matrix(matrix, rest_types)
                 }
            },
            _ => {
                // Int, Str, etc.
                self.check_wildcard_matrix(matrix, rest_types)
            }
        }
    }

    fn check_wildcard_matrix(&self, matrix: &[Vec<&Pattern>], rest_types: &[&Type]) -> Result<(), String> {
         // Filter rows starting with Wildcard/Variable.
         let mut new_matrix = Vec::new();
         for row in matrix {
             match row[0] {
                 Pattern::Wildcard | Pattern::Variable(..) => {
                     new_matrix.push(row[1..].to_vec());
                 },
                 _ => {}
             }
         }
         if new_matrix.is_empty() {
             return Err("Non-exhaustive: missing wildcard/variable case".to_string());
         }
         self.check_matrix(&new_matrix, rest_types)
    }

    fn check_constructor_matrix(&self, matrix: &[Vec<&Pattern>], ctor: &str, arity: usize, arg_types: &[&Type], rest_types: &[&Type]) -> Result<(), String> {
        let mut new_matrix = Vec::new();
        let mut new_types = arg_types.to_vec();
        new_types.extend_from_slice(rest_types);

        for row in matrix {
            let p = row[0];
            let rest = &row[1..];
            match p {
                 Pattern::Constructor(c, args) => {
                     if c == ctor {
                         let mut new_row: Vec<&Pattern> = args.iter().collect();
                         new_row.extend_from_slice(rest);
                         new_matrix.push(new_row);
                     }
                 },
                 Pattern::Literal(lit) => {
                     let name = match lit {
                         Literal::Bool(true) => "true",
                         Literal::Bool(false) => "false",
                         Literal::Unit => "()",
                         _ => "",
                     };
                     if name == ctor {
                         let mut new_row = Vec::new(); // arity 0
                         new_row.extend_from_slice(rest);
                         new_matrix.push(new_row);
                     }
                 },
                 Pattern::Wildcard | Pattern::Variable(..) => {
                     let mut new_row = vec![&Pattern::Wildcard; arity];
                     new_row.extend_from_slice(rest);
                     new_matrix.push(new_row);
                 },
                 _ => {}
            }
        }
        
        self.check_matrix(&new_matrix, &new_types).map_err(|_| format!("Non-exhaustive: missing {} case", ctor))
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
            (Type::Arrow(p1, r1, e1), Type::Arrow(p2, r2, e2)) => {
                if p1.len() != p2.len() {
                    return Err("Arity mismatch".into());
                }
                // Unify params
                let mut s = HashMap::new();
                for (a, b) in p1.iter().zip(p2) {
                    let s_new = self.unify(&apply_subst_type(&s, a), &apply_subst_type(&s, b))?;
                    s = compose_subst(&s, &s_new);
                }
                // Unify return type
                let s_ret = self.unify(&apply_subst_type(&s, r1), &apply_subst_type(&s, r2))?;
                s = compose_subst(&s, &s_ret);
                
                // Unify effects
                let s_eff = self.unify(&apply_subst_type(&s, e1), &apply_subst_type(&s, e2))?;
                s = compose_subst(&s, &s_eff);
                
                Ok(s)
            }
            (Type::Row(e1, t1), Type::Row(e2, t2)) => {
                let mut s = HashMap::new();
                let mut e2_remaining = e2.clone();
                let mut e1_remaining = Vec::new();

                for h1 in e1 {
                    let h1_sub = apply_subst_type(&s, h1);
                    if let Some(idx) = e2_remaining.iter().position(|h2| h1_sub == apply_subst_type(&s, h2)) {
                        e2_remaining.remove(idx);
                    } else {
                        e1_remaining.push(h1.clone());
                    }
                }

                let fresh_tail = if t1.is_some() || t2.is_some() || !e1_remaining.is_empty() || !e2_remaining.is_empty() {
                    Some(Box::new(self.new_var()))
                } else {
                    None
                };

                if let Some(t1_inner) = t1 {
                    let row = Type::Row(e2_remaining.clone(), fresh_tail.clone());
                    let s_new = self.unify(&apply_subst_type(&s, t1_inner), &row)?;
                    s = compose_subst(&s, &s_new);
                } else if !e2_remaining.is_empty() {
                    return Err(format!("Row mismatch: extra effects in RHS: {:?}", e2_remaining));
                }

                if let Some(t2_inner) = t2 {
                    let row = Type::Row(e1_remaining, fresh_tail);
                    let s_new = self.unify(&apply_subst_type(&s, t2_inner), &row)?;
                    s = compose_subst(&s, &s_new);
                } else if !e1_remaining.is_empty() {
                    return Err(format!("Row mismatch: extra effects in LHS: {:?}", e1_remaining));
                }

                Ok(s)
            }
            (Type::Record(f1), Type::Record(f2)) => {
                if f1.len() != f2.len() {
                    return Err("Record arity mismatch".into());
                }
                let mut f1_sorted = f1.clone();
                f1_sorted.sort_by(|a, b| a.0.cmp(&b.0));
                let mut f2_sorted = f2.clone();
                f2_sorted.sort_by(|a, b| a.0.cmp(&b.0));

                let mut s = HashMap::new();
                for ((n1, t1), (n2, t2)) in f1_sorted.iter().zip(f2_sorted.iter()) {
                    if n1 != n2 {
                        return Err(format!("Record field mismatch: {} vs {}", n1, n2));
                    }
                    let s_new = self.unify(&apply_subst_type(&s, t1), &apply_subst_type(&s, t2))?;
                    s = compose_subst(&s, &s_new);
                }
                Ok(s)
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
            (Type::Linear(t1), Type::Linear(t2)) => self.unify(t1, t2),
            _ => Err(format!("Type mismatch: {:?} vs {:?}", t1, t2)),
        }
    }
}

pub fn apply_subst_type(subst: &Subst, typ: &Type) -> Type {
    match typ {
        Type::Var(n) => subst.get(n).cloned().unwrap_or(typ.clone()),
        Type::Arrow(params, ret, effects) => Type::Arrow(
            params.iter().map(|p| apply_subst_type(subst, p)).collect(),
            Box::new(apply_subst_type(subst, ret)),
            Box::new(apply_subst_type(subst, effects)),
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
        Type::Linear(inner) => Type::Linear(Box::new(apply_subst_type(subst, inner))),
        Type::Row(effs, tail) => Type::Row(
            effs.iter().map(|e| apply_subst_type(subst, e)).collect(),
            tail.as_ref().map(|t| Box::new(apply_subst_type(subst, t))),
        ),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), apply_subst_type(subst, t)))
                .collect(),
        ),
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
        Type::Arrow(params, ret, effects) => {
            let mut s = get_free_vars_type(ret);
            for p in params {
                s.extend(get_free_vars_type(p));
            }
            s.extend(get_free_vars_type(effects));
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
        Type::Linear(inner) => get_free_vars_type(inner),
        Type::Row(effs, tail) => {
            let mut s = HashSet::new();
            for e in effs {
                s.extend(get_free_vars_type(e));
            }
            if let Some(t) = tail {
                s.extend(get_free_vars_type(t));
            }
            s
        }
        Type::Record(fields) => {
            let mut s = HashSet::new();
            for (_, t) in fields {
                s.extend(get_free_vars_type(t));
            }
            s
        }
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
        Type::Arrow(params, ret, effects) => {
            occurs_check(name, ret) || params.iter().any(|p| occurs_check(name, p)) || occurs_check(name, effects)
        }
        Type::UserDefined(_, args) => args.iter().any(|a| occurs_check(name, a)),
        Type::Result(ok, err) => occurs_check(name, ok) || occurs_check(name, err),
        Type::Ref(inner) => occurs_check(name, inner),
        Type::Linear(inner) => occurs_check(name, inner),
        Type::Row(effs, tail) => {
            effs.iter().any(|e| occurs_check(name, e)) || tail.as_ref().map_or(false, |t| occurs_check(name, t))
        }
        Type::Record(fields) => fields.iter().any(|(_, t)| occurs_check(name, t)),
        _ => false,
    }
}

fn contains_ref(typ: &Type) -> bool {
    match typ {
        Type::Ref(_) => true,
        Type::Arrow(params, ret, effects) => contains_ref(ret) || params.iter().any(contains_ref) || contains_ref(effects),
        Type::UserDefined(_, args) => args.iter().any(contains_ref),
        Type::Result(ok, err) => contains_ref(ok) || contains_ref(err),
        Type::Linear(inner) => contains_ref(inner),
        Type::Row(effs, tail) => {
            effs.iter().any(contains_ref) || tail.as_ref().map_or(false, |t| contains_ref(t))
        }
        Type::Record(fields) => fields.iter().any(|(_, t)| contains_ref(t)),
        _ => false,
    }
}

fn contains_linear(typ: &Type) -> bool {
    match typ {
        Type::Linear(_) => true,
        Type::Ref(inner) => contains_linear(inner),
        Type::Arrow(params, ret, effects) => contains_linear(ret) || params.iter().any(contains_linear) || contains_linear(effects),
        Type::UserDefined(_, args) => args.iter().any(contains_linear),
        Type::Result(ok, err) => contains_linear(ok) || contains_linear(err),
        Type::Row(effs, tail) => {
            effs.iter().any(contains_linear) || tail.as_ref().map_or(false, |t| contains_linear(t))
        }
        Type::Record(fields) => fields.iter().any(|(_, t)| contains_linear(t)),
        _ => false,
    }
}
