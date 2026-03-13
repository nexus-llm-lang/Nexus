mod capture;
mod env;
mod exhaustiveness;
mod helpers;
mod lint;
mod unify;

pub use env::{Scheme, TypeEnv, TypeError, TypeWarning};
pub use helpers::{exn_enum_def, list_enum_def};
pub use unify::apply_subst_type;

use capture::{collect_lambda_captures, lambda_references_name};
use helpers::{
    check_unintroduced_type_vars, contains_exn_throws, contains_ref, contains_return,
    default_numeric_literals, describe_ctor_field, external_scheme, extract_row_port_names,
    get_default_alias, is_allowed_main_require_signature, is_allowed_main_throws_signature,
    is_auto_droppable, merge_type_rows, normalize_enum_generic_params,
    normalize_typedef_generic_params, register_exception_variant,
    register_nullary_variant_constructor, register_stdlib_types, select_float_type,
    select_int_type, strip_required_port_coeffect, summarize_ctor_args, summarize_ctor_fields,
};
use lint::{
    collect_signature_needs_from_stmts, extract_named_row_members,
    find_private_type_in_public_signature,
};
use unify::compose_subst;

use super::ast::*;
use super::parser;
use crate::constants::ENTRYPOINT;
use crate::lang::stdlib::resolve_import_path;
use std::collections::{HashMap, HashSet};
use std::fs;

const THROWS_EXN: &str = "Exn";

type Subst = HashMap<String, Type>;

pub struct TypeChecker {
    pub supply: usize,
    pub env: TypeEnv,
    pub visited_paths: HashSet<String>,
    pub import_cache: HashMap<String, TypeEnv>,
    pub warnings: Vec<TypeWarning>,
    /// Persistent copy of type definitions for use in `unify`,
    /// which runs while `self.env` is temporarily taken.
    type_defs: HashMap<String, TypeDef>,
}

impl TypeChecker {
    /// Creates a checker with only language-core builtins (no stdlib `.nx` imports).
    pub fn new_without_stdlib() -> Self {
        let mut env = TypeEnv::new();
        env.enums.insert(THROWS_EXN.to_string(), exn_enum_def());
        env.enums.insert("List".to_string(), list_enum_def());
        register_nullary_variant_constructor(
            &mut env,
            "List",
            &["T".to_string()],
            &list_enum_def().variants[0],
        );

        env.linear_vars.clear();
        TypeChecker {
            supply: 0,
            type_defs: env.types.clone(),
            env,
            visited_paths: HashSet::new(),
            import_cache: HashMap::new(),
            warnings: Vec::new(),
        }
    }

    /// Creates a checker with stdlib types (List, etc.) but no stdlib functions.
    /// Stdlib functions must be imported explicitly via `import` statements.
    pub fn new() -> Self {
        let mut checker = Self::new_without_stdlib();
        register_stdlib_types(&mut checker.env);
        checker
    }

    pub fn take_warnings(&mut self) -> Vec<TypeWarning> {
        std::mem::take(&mut self.warnings)
    }

    pub(crate) fn new_var(&mut self) -> Type {
        let n = self.supply;
        self.supply += 1;
        Type::Var(format!("?{}", n))
    }

    /// Type-checks a full program and updates internal environment state.
    #[tracing::instrument(skip_all, name = "typecheck")]
    pub fn check_program(&mut self, program: &Program) -> Result<(), TypeError> {
        self.warnings.clear();
        // Pass 1: Collect imports, types, enums, exceptions, ports, and signatures of global lets
        for def in &program.definitions {
            match &def.node {
                TopLevel::Import(import) => {
                    if !import.is_external {
                        if self.visited_paths.contains(&import.path) {
                            // Reuse cached public_env for named imports from same path.
                            if let Some(cached) = self.import_cache.get(&import.path) {
                                if !import.items.is_empty() {
                                    let public_env = cached.clone();
                                    for item in &import.items {
                                        let mut imported_any = false;
                                        if let Some(sch) = public_env.vars.get(item) {
                                            self.env.insert(item.clone(), sch.clone());
                                            imported_any = true;
                                        }
                                        if let Some(td) = public_env.types.get(item) {
                                            self.env.types.insert(item.clone(), td.clone());
                                            self.type_defs.insert(item.clone(), td.clone());
                                            imported_any = true;
                                        }
                                        if let Some(ed) = public_env.enums.get(item) {
                                            self.env.enums.insert(item.clone(), ed.clone());
                                            for v in &ed.variants {
                                                register_nullary_variant_constructor(
                                                    &mut self.env,
                                                    &ed.name,
                                                    &ed.type_params,
                                                    v,
                                                );
                                            }
                                            imported_any = true;
                                        }
                                        let port_prefix = format!("{}.", item);
                                        let port_items: Vec<(String, Scheme)> = public_env
                                            .vars
                                            .iter()
                                            .filter(|(name, _)| name.starts_with(&port_prefix))
                                            .map(|(name, sch)| (name.clone(), sch.clone()))
                                            .collect();
                                        if !port_items.is_empty() {
                                            for (name, sch) in port_items {
                                                self.env.insert(name, sch);
                                            }
                                            imported_any = true;
                                        }
                                        if !imported_any {
                                            return Err(TypeError {
                                                message: format!(
                                                    "Definition {} not found in {}",
                                                    item, import.path
                                                ),
                                                span: def.span.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                            continue;
                        }
                        self.visited_paths.insert(import.path.clone());

                        let resolved_path = resolve_import_path(&import.path);
                        let src = fs::read_to_string(&resolved_path).map_err(|e| TypeError {
                            message: format!("Failed to read {}: {}", resolved_path, e),
                            span: def.span.clone(),
                        })?;
                        let p = parser::parser().parse(&src).map_err(|_| TypeError {
                            message: format!("Failed to parse {}", import.path),
                            span: def.span.clone(),
                        })?;

                        let mut sub_checker = TypeChecker::new();
                        sub_checker.visited_paths = self.visited_paths.clone();
                        sub_checker.import_cache = self.import_cache.clone();
                        sub_checker.check_program(&p)?;

                        let mut public_env = TypeEnv::new();
                        for sub_def in &p.definitions {
                            match &sub_def.node {
                                TopLevel::TypeDef(td) if td.is_public => {
                                    let td_norm = normalize_typedef_generic_params(td);
                                    public_env.types.insert(td_norm.name.clone(), td_norm);
                                }
                                TopLevel::Enum(ed) if ed.is_public => {
                                    if ed.name == "Exn" {
                                        return Err(TypeError {
                                            message:
                                                "Reserved enum name 'Exn'; use 'exception ...' declarations"
                                                    .into(),
                                            span: sub_def.span.clone(),
                                        });
                                    }
                                    let ed_norm = normalize_enum_generic_params(ed);
                                    if ed.is_opaque {
                                        // Opaque: export type name only, no constructors or variants
                                        let opaque_ed = EnumDef {
                                            name: ed_norm.name.clone(),
                                            is_public: true,
                                            is_opaque: true,
                                            type_params: ed_norm.type_params.clone(),
                                            variants: vec![],
                                        };
                                        public_env.enums.insert(opaque_ed.name.clone(), opaque_ed);
                                    } else {
                                        public_env
                                            .enums
                                            .insert(ed_norm.name.clone(), ed_norm.clone());
                                        for v in &ed_norm.variants {
                                            register_nullary_variant_constructor(
                                                &mut public_env,
                                                &ed_norm.name,
                                                &ed_norm.type_params,
                                                v,
                                            );
                                        }
                                    }
                                }
                                TopLevel::Exception(ex) if ex.is_public => {
                                    register_exception_variant(&mut public_env, ex, &sub_def.span)?;
                                }
                                TopLevel::Let(gl) if gl.is_public => {
                                    if let Some(sch) = sub_checker.env.vars.get(&gl.name) {
                                        public_env.insert(gl.name.clone(), sch.clone());
                                    }
                                }
                                TopLevel::Port(port) if port.is_public => {
                                    // Export all port operation signatures (e.g. Net.get, Net.listen)
                                    let prefix = format!("{}.", port.name);
                                    for (name, sch) in &sub_checker.env.vars {
                                        if name.starts_with(&prefix) {
                                            public_env.insert(name.clone(), sch.clone());
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        self.visited_paths = sub_checker.visited_paths;
                        for (k, v) in sub_checker.import_cache {
                            self.import_cache.entry(k).or_insert(v);
                        }
                        self.import_cache
                            .insert(import.path.clone(), public_env.clone());

                        if !import.items.is_empty() {
                            for item in &import.items {
                                let mut imported_any = false;

                                if let Some(sch) = public_env.vars.get(item) {
                                    self.env.insert(item.clone(), sch.clone());
                                    imported_any = true;
                                }

                                if let Some(td) = public_env.types.get(item) {
                                    self.env.types.insert(item.clone(), td.clone());
                                    self.type_defs.insert(item.clone(), td.clone());
                                    imported_any = true;
                                }

                                if let Some(ed) = public_env.enums.get(item) {
                                    self.env.enums.insert(item.clone(), ed.clone());
                                    for v in &ed.variants {
                                        register_nullary_variant_constructor(
                                            &mut self.env,
                                            &ed.name,
                                            &ed.type_params,
                                            v,
                                        );
                                    }
                                    imported_any = true;
                                }

                                // Selective import of a port namespace imports all `Port.fn` entries.
                                let port_prefix = format!("{}.", item);
                                let port_items: Vec<(String, Scheme)> = public_env
                                    .vars
                                    .iter()
                                    .filter(|(name, _)| name.starts_with(&port_prefix))
                                    .map(|(name, sch)| (name.clone(), sch.clone()))
                                    .collect();
                                if !port_items.is_empty() {
                                    for (name, sch) in port_items {
                                        self.env.insert(name, sch);
                                    }
                                    imported_any = true;
                                }

                                if !imported_any {
                                    return Err(TypeError {
                                        message: format!(
                                            "Definition {} not found in {}",
                                            item, import.path
                                        ),
                                        span: def.span.clone(),
                                    });
                                }
                            }
                        }
                        if import.alias.is_some() || import.items.is_empty() {
                            let alias = import
                                .alias
                                .clone()
                                .unwrap_or_else(|| get_default_alias(&import.path));
                            self.env.modules.insert(alias, public_env);
                        }
                    }
                }
                TopLevel::TypeDef(td) => {
                    let td_norm = normalize_typedef_generic_params(td);
                    self.type_defs.insert(td_norm.name.clone(), td_norm.clone());
                    self.env.types.insert(td_norm.name.clone(), td_norm);
                }
                TopLevel::Enum(ed) => {
                    if ed.name == "Exn" {
                        return Err(TypeError {
                            message: "Reserved enum name 'Exn'; use 'exception ...' declarations"
                                .into(),
                            span: def.span.clone(),
                        });
                    }
                    let ed_norm = normalize_enum_generic_params(ed);
                    self.env.enums.insert(ed_norm.name.clone(), ed_norm.clone());
                    for v in &ed_norm.variants {
                        register_nullary_variant_constructor(
                            &mut self.env,
                            &ed_norm.name,
                            &ed_norm.type_params,
                            v,
                        );
                    }
                }
                TopLevel::Exception(ex) => {
                    register_exception_variant(&mut self.env, ex, &def.span)?;
                }
                TopLevel::Port(port) => {
                    for sig in &port.functions {
                        let name = format!("{}.{}", port.name, sig.name);
                        let ptypes: Vec<(String, Type)> = sig
                            .params
                            .iter()
                            .map(|p| (p.name.clone(), p.typ.clone()))
                            .collect();
                        let port_req = Type::UserDefined(port.name.clone(), vec![]);
                        let requires = match &sig.requires {
                            Type::Row(reqs, tail) => {
                                let mut merged = reqs.clone();
                                if !merged.contains(&port_req) {
                                    merged.insert(0, port_req);
                                }
                                Type::Row(merged, tail.clone())
                            }
                            other => Type::Row(vec![port_req], Some(Box::new(other.clone()))),
                        };
                        self.env.insert(
                            name,
                            Scheme {
                                vars: vec![],
                                typ: Type::Arrow(
                                    ptypes,
                                    Box::new(sig.ret_type.clone()),
                                    Box::new(requires),
                                    // Ports are coeffects (environment requirements), not builtin throws.
                                    Box::new(sig.throws.clone()),
                                ),
                            },
                        );
                    }
                }
                TopLevel::Let(gl) => {
                    // Pre-register function signatures for recursion
                    match &gl.value.node {
                        Expr::Lambda {
                            type_params,
                            params,
                            ret_type,
                            requires,
                            throws,
                            ..
                        } => {
                            let vars_set: HashSet<String> = type_params.iter().cloned().collect();
                            self.env.insert(
                                gl.name.clone(),
                                Scheme {
                                    vars: type_params.clone(),
                                    typ: Type::Arrow(
                                        params
                                            .iter()
                                            .map(|p| {
                                                (
                                                    p.name.clone(),
                                                    self.convert_user_defined_to_var(
                                                        &p.typ, &vars_set,
                                                    ),
                                                )
                                            })
                                            .collect(),
                                        Box::new(
                                            self.convert_user_defined_to_var(ret_type, &vars_set),
                                        ),
                                        Box::new(
                                            self.convert_user_defined_to_var(requires, &vars_set),
                                        ),
                                        Box::new(
                                            self.convert_user_defined_to_var(throws, &vars_set),
                                        ),
                                    ),
                                },
                            );
                        }
                        Expr::External(_, type_params, typ) => {
                            let vars_set: HashSet<String> = type_params.iter().cloned().collect();
                            check_unintroduced_type_vars(typ, &vars_set, &self.env).map_err(
                                |e| TypeError {
                                    message: e,
                                    span: gl.value.span.clone(),
                                },
                            )?;
                            let scheme = external_scheme(type_params, typ);
                            self.env.insert(gl.name.clone(), scheme);
                        }
                        _ => {}
                    }
                }
            }
        }

        // Pass 2: Check all global let bodies and handlers
        self.env.linear_vars.clear();
        for def in &program.definitions {
            match &def.node {
                TopLevel::Let(gl) => {
                    // External bindings already have their scheme from pass 1; skip re-inference.
                    if matches!(&gl.value.node, Expr::External(_, _, _)) {
                        continue;
                    }
                    let v = self.new_var();
                    let mut env = std::mem::take(&mut self.env);
                    let empty_req = Type::Row(vec![], None);
                    let res = self.infer(&mut env, &gl.value, &Type::Unit, &empty_req, &v);
                    self.env = env;
                    let (s, t) = res?;
                    let mut t = apply_subst_type(&s, &t);
                    if let Some(ann) = &gl.typ {
                        let sa = self.unify(&t, ann).map_err(|e| TypeError {
                            message: e,
                            span: gl.value.span.clone(),
                        })?;
                        t = apply_subst_type(&sa, ann);
                    } else {
                        t = default_numeric_literals(&t);
                    }

                    if gl.name == ENTRYPOINT {
                        if gl.is_public {
                            return Err(TypeError {
                                message: "main function must be private (remove 'pub')".into(),
                                span: def.span.clone(),
                            });
                        }
                        // Accept main() -> unit or main(args: [string]) -> unit
                        let (sm, req, ef) = {
                            let req0 = self.new_var();
                            let ef0 = self.new_var();
                            if let Ok(sm) = self.unify(
                                &t,
                                &Type::Arrow(
                                    vec![],
                                    Box::new(Type::Unit),
                                    Box::new(req0.clone()),
                                    Box::new(ef0.clone()),
                                ),
                            ) {
                                (sm, req0, ef0)
                            } else {
                                let req1 = self.new_var();
                                let ef1 = self.new_var();
                                let sm = self
                                    .unify(
                                        &t,
                                        &Type::Arrow(
                                            vec![("args".to_string(), Type::List(Box::new(Type::String)))],
                                            Box::new(Type::Unit),
                                            Box::new(req1.clone()),
                                            Box::new(ef1.clone()),
                                        ),
                                    )
                                    .map_err(|_| TypeError {
                                        message: "main must be '() -> unit' or '(args: [string]) -> unit'".into(),
                                        span: def.span.clone(),
                                    })?;
                                (sm, req1, ef1)
                            }
                        };
                        let final_req = apply_subst_type(&sm, &req);
                        if !is_allowed_main_require_signature(&final_req) {
                            return Err(TypeError {
                                message:
                                    "main function requires must be {}, or a subset of { PermFs, PermNet, PermConsole, PermRandom, PermClock, PermProc }"
                                        .into(),
                                span: def.span.clone(),
                            });
                        }
                        let final_ef = apply_subst_type(&sm, &ef);
                        if contains_exn_throws(&final_ef) {
                            return Err(TypeError {
                                message: "main function must not declare Exn in throws".into(),
                                span: def.span.clone(),
                            });
                        }
                        if !is_allowed_main_throws_signature(&final_ef) {
                            return Err(TypeError {
                                message: "main function throws must be {}".into(),
                                span: def.span.clone(),
                            });
                        }
                    }

                    if gl.is_public {
                        if let Some(private_type_name) =
                            find_private_type_in_public_signature(&t, &self.env)
                        {
                            return Err(TypeError {
                                message: format!(
                                    "public definition '{}' exposes private type '{}'",
                                    gl.name, private_type_name
                                ),
                                span: def.span.clone(),
                            });
                        }
                    }

                    self.env
                        .insert(gl.name.clone(), self.generalize(&self.env, t));
                }
                _ => {}
            }
        }
        self.check_missing_requirements(program)?;
        self.collect_lint_warnings(program);
        Ok(())
    }

    fn check_missing_requirements(&self, program: &Program) -> Result<(), TypeError> {
        for def in &program.definitions {
            let TopLevel::Let(gl) = &def.node else {
                continue;
            };
            let Expr::Lambda { requires, body, .. } = &gl.value.node else {
                continue;
            };

            let (used_reqs, _, unknown) = collect_signature_needs_from_stmts(body, &self.env);
            if unknown {
                continue;
            }
            let (declared_reqs, req_unknown) = extract_named_row_members(requires);
            if req_unknown {
                continue;
            }
            let mut missing: Vec<String> = used_reqs.difference(&declared_reqs).cloned().collect();
            missing.sort();
            if !missing.is_empty() {
                return Err(TypeError {
                    message: format!(
                        "Function '{}' uses coeffects [{}] not declared in its require clause",
                        gl.name,
                        missing.join(", ")
                    ),
                    span: def.span.clone(),
                });
            }
        }
        Ok(())
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
            Type::Arrow(p, r, req, e) => Type::Arrow(
                p.iter()
                    .map(|(n, t)| (n.clone(), self.convert_user_defined_to_var(t, vars)))
                    .collect(),
                Box::new(self.convert_user_defined_to_var(r, vars)),
                Box::new(self.convert_user_defined_to_var(req, vars)),
                Box::new(self.convert_user_defined_to_var(e, vars)),
            ),
            Type::Ref(i) => Type::Ref(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Linear(i) => Type::Linear(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Borrow(i) => Type::Borrow(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Array(i) => Type::Array(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::List(i) => Type::List(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Row(es, t) => Type::Row(
                es.iter()
                    .map(|x| self.convert_user_defined_to_var(x, vars))
                    .collect(),
                t.as_ref()
                    .map(|x| Box::new(self.convert_user_defined_to_var(x, vars))),
            ),
            Type::Record(fs) => Type::Record(
                fs.iter()
                    .map(|(n, t)| (n.clone(), self.convert_user_defined_to_var(t, vars)))
                    .collect(),
            ),
            _ => typ.clone(),
        }
    }

    /// Check a function body. `extra_requires` is merged into the function's
    /// declared requires -- used for handler method bodies so that the
    /// handler-level `require` clause is visible inside each method.
    fn check_function(
        &mut self,
        func: &Function,
        base_env: &TypeEnv,
        span: &Span,
        extra_requires: &Type,
    ) -> Result<(), TypeError> {
        let mut env = base_env.clone();
        for p in &func.params {
            env.insert(
                p.sigil.get_key(&p.name),
                Scheme {
                    vars: vec![],
                    typ: p.typ.clone(),
                },
            );
        }
        if contains_ref(&func.ret_type) {
            return Err(TypeError {
                message: "Cannot return Ref".into(),
                span: span.clone(),
            });
        }
        let merged_requires = merge_type_rows(&func.requires, extra_requires);
        self.infer_body(
            &func.body,
            &mut env,
            &func.ret_type,
            &merged_requires,
            &func.throws,
        )?;
        if !contains_return(&func.body) && !matches!(func.ret_type, Type::Unit) {
            return Err(TypeError {
                message: "Function body has no return statement; implicit return type is Unit"
                    .into(),
                span: span.clone(),
            });
        }
        env.check_unused_linear(span)?;
        Ok(())
    }

    fn infer_body(
        &mut self,
        body: &[Spanned<Stmt>],
        env: &mut TypeEnv,
        er: &Type,
        eq: &Type,
        ee: &Type,
    ) -> Result<(), TypeError> {
        for s in body {
            match &s.node {
                Stmt::Let {
                    name,
                    sigil,
                    typ,
                    value,
                } => {
                    let is_recursive_lambda = matches!(&value.node, Expr::Lambda { params, body, .. }
                        if lambda_references_name(body, params, name));
                    let key = sigil.get_key(name);
                    if is_recursive_lambda {
                        if !matches!(sigil, Sigil::Immutable) {
                            return Err(TypeError {
                                message: "Recursive lambda binding must be immutable".into(),
                                span: value.span.clone(),
                            });
                        }
                        let ann = if let Some(t) = typ.clone() {
                            t
                        } else if let Expr::Lambda {
                            type_params,
                            params,
                            ret_type,
                            requires,
                            throws,
                            ..
                        } = &value.node
                        {
                            let vars_set: HashSet<String> =
                                type_params.iter().cloned().collect();
                            Type::Arrow(
                                params
                                    .iter()
                                    .map(|p| {
                                        (
                                            p.name.clone(),
                                            self.convert_user_defined_to_var(
                                                &p.typ, &vars_set,
                                            ),
                                        )
                                    })
                                    .collect(),
                                Box::new(
                                    self.convert_user_defined_to_var(ret_type, &vars_set),
                                ),
                                Box::new(
                                    self.convert_user_defined_to_var(requires, &vars_set),
                                ),
                                Box::new(
                                    self.convert_user_defined_to_var(throws, &vars_set),
                                ),
                            )
                        } else {
                            return Err(TypeError {
                                message:
                                    "Recursive lambda requires an explicit type annotation"
                                        .into(),
                                span: value.span.clone(),
                            });
                        };
                        env.insert(
                            key.clone(),
                            Scheme {
                                vars: vec![],
                                typ: ann,
                            },
                        );
                    }
                    let (s1, t1) = self.infer(env, value, er, eq, ee)?;
                    env.apply(&s1);
                    let mut t1 = apply_subst_type(&s1, &t1);
                    if let Some(ann) = typ {
                        let sa = self.unify(&t1, ann).map_err(|e| TypeError {
                            message: e,
                            span: value.span.clone(),
                        })?;
                        env.apply(&sa);
                        t1 = apply_subst_type(&sa, ann);
                    } else {
                        t1 = default_numeric_literals(&t1);
                    }
                    let ft = match sigil {
                        Sigil::Mutable => {
                            if env.contains_linear_type(&t1) {
                                return Err(TypeError {
                                    message: "Mutable linear".into(),
                                    span: value.span.clone(),
                                });
                            }
                            Type::Ref(Box::new(t1))
                        }
                        Sigil::Linear => {
                            if is_auto_droppable(&t1) {
                                self.warnings.push(TypeWarning {
                                    message: format!(
                                        "Linear sigil '%' on '{}' is unnecessary: primitive type '{:?}' is automatically managed",
                                        name,
                                        t1,
                                    ),
                                    span: value.span.clone(),
                                });
                            }
                            match t1 {
                                Type::Linear(_) => t1,
                                _ => Type::Linear(Box::new(t1)),
                            }
                        }
                        Sigil::Immutable => {
                            if contains_ref(&t1) {
                                return Err(TypeError {
                                    message: "Immutable Ref".into(),
                                    span: value.span.clone(),
                                });
                            }
                            t1
                        }
                        Sigil::Borrow => {
                            let inner = match t1 {
                                Type::Linear(t) | Type::Borrow(t) | Type::Ref(t) => t,
                                _ => Box::new(t1),
                            };
                            Type::Borrow(inner)
                        }
                    };
                    env.insert(key, self.generalize(env, ft));
                }
                Stmt::Return(e) => {
                    let (s1, t1) = self.infer(env, e, er, eq, ee)?;
                    env.apply(&s1);
                    env.check_unused_linear(&e.span)?;
                    self.unify(&t1, &apply_subst_type(&s1, er))
                        .map_err(|err| TypeError {
                            message: err,
                            span: e.span.clone(),
                        })?;
                }
                Stmt::Expr(e) => {
                    self.infer(env, e, er, eq, ee)?;
                }
                Stmt::Assign { target, value } => {
                    let (s_v, t_v) = self.infer(env, value, er, eq, ee)?;
                    env.apply(&s_v);
                    match &target.node {
                        Expr::Variable(name, sigil) => {
                            if let Sigil::Immutable = sigil {
                                return Err(TypeError {
                                    message: "Mutating immutable".into(),
                                    span: s.span.clone(),
                                });
                            }
                            if let Some(sch) = env.get(&sigil.get_key(name)) {
                                if let Type::Ref(i) = self.instantiate(sch) {
                                    self.unify(&t_v, &i).map_err(|e| TypeError {
                                        message: e,
                                        span: value.span.clone(),
                                    })?;
                                } else {
                                    return Err(TypeError {
                                        message: "Not a ref".into(),
                                        span: s.span.clone(),
                                    });
                                }
                            } else {
                                return Err(TypeError {
                                    message: "Not found".into(),
                                    span: s.span.clone(),
                                });
                            }
                        }
                        Expr::Index(arr, idx) => {
                            // Typecheck index
                            let (s_idx, t_idx) = self.infer(env, idx, er, eq, ee)?;
                            env.apply(&s_idx);
                            self.unify(&t_idx, &Type::I64).map_err(|e| TypeError {
                                message: e,
                                span: idx.span.clone(),
                            })?;

                            // Typecheck array WITHOUT consuming if it's a variable
                            let t_arr = match &arr.node {
                                Expr::Variable(n, s) => {
                                    let key = s.get_key(n);
                                    if let Some(sch) = env.get(&key) {
                                        self.instantiate(sch)
                                    } else {
                                        return Err(TypeError {
                                            message: format!("Not found: {}", key),
                                            span: arr.span.clone(),
                                        });
                                    }
                                }
                                _ => {
                                    let (s_a, t_a) = self.infer(env, arr, er, eq, ee)?;
                                    env.apply(&s_a);
                                    t_a
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
                                    _ => {
                                        return Err(TypeError {
                                            message: "Not an array".into(),
                                            span: arr.span.clone(),
                                        })
                                    }
                                },
                                _ => {
                                    return Err(TypeError {
                                        message: "Not an array".into(),
                                        span: arr.span.clone(),
                                    })
                                }
                            };
                            self.unify(&t_v, &elem_t).map_err(|e| TypeError {
                                message: e,
                                span: value.span.clone(),
                            })?;
                        }
                        _ => {
                            return Err(TypeError {
                                message: "Invalid assignment target".into(),
                                span: s.span.clone(),
                            })
                        }
                    }
                }
                Stmt::Conc(ts) => {
                    for t in ts {
                        self.check_task(t, env, &s.span, eq)?;
                    }
                }
                Stmt::Try {
                    body,
                    catch_param,
                    catch_body,
                } => {
                    let exn = Type::UserDefined("Exn".into(), vec![]);
                    let try_eff = Type::Row(vec![exn], Some(Box::new(ee.clone())));
                    let mut et = env.clone();
                    self.infer_body(body, &mut et, er, eq, &try_eff)?;
                    let mut ec = env.clone();
                    ec.insert(
                        catch_param.clone(),
                        Scheme {
                            vars: vec![],
                            typ: Type::UserDefined("Exn".into(), vec![]),
                        },
                    );
                    self.infer_body(catch_body, &mut ec, er, eq, ee)?;
                    if et.linear_vars != ec.linear_vars {
                        return Err(TypeError {
                            message: "Linear mismatch".into(),
                            span: s.span.clone(),
                        });
                    }
                    env.linear_vars = et.linear_vars;
                }
                Stmt::Inject { handlers, body } => {
                    let mut injected_reqs = Vec::new();
                    let mut injected_port_names = HashSet::new();
                    let mut handler_extra_reqs = HashSet::new();
                    for handler_name in handlers {
                        let Some(scheme) = env.get(handler_name).cloned() else {
                            return Err(TypeError {
                                message: format!("Handler '{}' not found in scope", handler_name),
                                span: s.span.clone(),
                            });
                        };
                        let instantiated = self.instantiate(&scheme);
                        match instantiated {
                            Type::Handler(port_name, handler_req) => {
                                injected_port_names.insert(port_name.clone());
                                let req = Type::UserDefined(port_name, vec![]);
                                if !injected_reqs.contains(&req) {
                                    injected_reqs.push(req);
                                }
                                // Collect handler's require coeffects to propagate to caller
                                for r in extract_row_port_names(&handler_req) {
                                    handler_extra_reqs.insert(r);
                                }
                            }
                            _ => {
                                return Err(TypeError {
                                    message: format!(
                                        "'{}' is not a handler value (expected type 'handler <Port>')",
                                        handler_name
                                    ),
                                    span: s.span.clone(),
                                });
                            }
                        }
                    }
                    let (body_reqs, _, body_unknown) =
                        collect_signature_needs_from_stmts(body, env);
                    if !body_unknown {
                        let mut non_reducing_handlers: Vec<String> = injected_port_names
                            .iter()
                            .filter(|port_name| !body_reqs.contains(*port_name))
                            .cloned()
                            .collect();
                        non_reducing_handlers.sort();
                        if !non_reducing_handlers.is_empty() {
                            return Err(TypeError {
                                message: format!(
                                    "Inject handler(s) {} does not reduce requirements in this scope",
                                    non_reducing_handlers.join(", ")
                                ),
                                span: s.span.clone(),
                            });
                        }
                    }
                    // Build the inject requirement: body's ports + handler extra requires
                    let mut all_inject_reqs = injected_reqs;
                    for extra in &handler_extra_reqs {
                        let extra_req = Type::UserDefined(extra.clone(), vec![]);
                        if !all_inject_reqs.contains(&extra_req) {
                            all_inject_reqs.push(extra_req);
                        }
                    }
                    let injected_eq = match eq {
                        Type::Row(reqs, tail) => {
                            let mut merged = reqs.clone();
                            for req in all_inject_reqs {
                                if !merged.contains(&req) {
                                    merged.push(req);
                                }
                            }
                            Type::Row(merged, tail.clone())
                        }
                        Type::Unit => Type::Row(all_inject_reqs, None),
                        other => Type::Row(all_inject_reqs, Some(Box::new(other.clone()))),
                    };
                    self.infer_body(body, env, er, &injected_eq, ee)?;
                }
                Stmt::LetPattern { pattern, value } => {
                    let (s1, t1) = self.infer(env, value, er, eq, ee)?;
                    env.apply(&s1);
                    let t1 = default_numeric_literals(&apply_subst_type(&s1, &t1));
                    // Reuse match exhaustiveness check with a single-case match
                    let dummy_case = MatchCase {
                        pattern: pattern.clone(),
                        body: vec![],
                    };
                    self.check_exhaustiveness(env, &t1, &[dummy_case])
                        .map_err(|m| TypeError {
                            message: format!("Non-exhaustive destructuring pattern: {}", m),
                            span: pattern.span.clone(),
                        })?;
                    let sp = self.bind_pattern(pattern, &t1, env)?;
                    env.apply(&sp);
                }
            }
        }
        Ok(())
    }

    fn check_task(
        &mut self,
        t: &Function,
        oe: &TypeEnv,
        _span: &Span,
        outer_eq: &Type,
    ) -> Result<(), TypeError> {
        let mut te = TypeEnv::new();
        te.types = oe.types.clone();
        te.enums = oe.enums.clone();
        let mut captured_linear = HashSet::new();
        for (k, s) in &oe.vars {
            if !k.starts_with('~') {
                te.insert(k.clone(), s.clone());
                if te.contains_linear_type(&s.typ) {
                    captured_linear.insert(k.clone());
                }
            }
        }
        let merged_requires = merge_type_rows(&t.requires, outer_eq);
        self.infer_body(&t.body, &mut te, &Type::Unit, &merged_requires, &t.throws)?;

        let unused_local_linear: Vec<_> = te
            .linear_vars
            .iter()
            .filter(|k| !captured_linear.contains(*k))
            .filter(|k| {
                if let Some(sch) = te.vars.get(*k) {
                    !is_auto_droppable(&sch.typ)
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        if !unused_local_linear.is_empty() {
            return Err(TypeError {
                message: format!("Unused linear in task: {:?}", unused_local_linear),
                span: _span.clone(),
            });
        }
        Ok(())
    }

    /// Type-checks a single REPL statement against the current checker state.
    pub fn check_repl_stmt(&mut self, s: &Spanned<Stmt>) -> Result<Type, TypeError> {
        let mut env = std::mem::replace(&mut self.env, TypeEnv::new());
        let res = (|| {
            let ev = self.new_var();
            let rq = self.new_var();
            match &s.node {
                Stmt::Expr(e) => {
                    let (sub, t) = self.infer(&mut env, e, &Type::Unit, &rq, &ev)?;
                    env.apply(&sub);
                    Ok(default_numeric_literals(&apply_subst_type(&sub, &t)))
                }
                _ => {
                    self.infer_body(&[s.clone()], &mut env, &Type::Unit, &rq, &ev)?;
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
        e: &Spanned<Expr>,
        er: &Type,
        eq: &Type,
        ee: &Type,
    ) -> Result<(Subst, Type), TypeError> {
        match &e.node {
            Expr::Literal(l) => Ok((
                HashMap::new(),
                match l {
                    Literal::Int(_) => Type::IntLit,
                    Literal::Float(_) => Type::FloatLit,
                    Literal::Bool(_) => Type::Bool,
                    Literal::Char(_) => Type::Char,
                    Literal::String(_) => Type::String,
                    Literal::Unit => Type::Unit,
                },
            )),
            Expr::Variable(n, s) => {
                let key = s.get_key(n);
                if let Some(sch) = env.get(&key).cloned() {
                    let mut t = self.instantiate(&sch);
                    if let Sigil::Mutable = s {
                        if let Type::Ref(i) = t {
                            t = *i;
                        }
                    }
                    if env.contains_linear_type(&t) {
                        env.consume(&key).map_err(|m| TypeError {
                            message: m,
                            span: e.span.clone(),
                        })?;
                    }
                    Ok((HashMap::new(), t))
                } else {
                    Err(TypeError {
                        message: format!("Not found: {}", key),
                        span: e.span.clone(),
                    })
                }
            }
            Expr::BinaryOp(l, op, r) => {
                let (s1, t1) = self.infer(env, l, er, eq, ee)?;
                let (s2, t2) = self.infer(env, r, er, eq, ee)?;
                let mut s = compose_subst(&s1, &s2);
                match op {
                    BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Shl
                    | BinaryOp::Shr => {
                        let lt = apply_subst_type(&s, &t1);
                        let rt = apply_subst_type(&s, &t2);
                        let target = select_int_type(&lt, &rt).ok_or_else(|| TypeError {
                            message: format!("Integer op expects i32/i64, got {} and {}", lt, rt),
                            span: e.span.clone(),
                        })?;

                        let s3 = self.unify(&lt, &target).map_err(|m| TypeError {
                            message: m,
                            span: l.span.clone(),
                        })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self
                            .unify(&apply_subst_type(&s, &t2), &target)
                            .map_err(|m| TypeError {
                                message: m,
                                span: r.span.clone(),
                            })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, target))
                    }
                    BinaryOp::Concat => {
                        let s3 = self
                            .unify(&apply_subst_type(&s, &t1), &Type::String)
                            .map_err(|m| TypeError {
                                message: m,
                                span: l.span.clone(),
                            })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self
                            .unify(&apply_subst_type(&s, &t2), &Type::String)
                            .map_err(|m| TypeError {
                                message: m,
                                span: r.span.clone(),
                            })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::String))
                    }
                    BinaryOp::FAdd | BinaryOp::FSub | BinaryOp::FMul | BinaryOp::FDiv => {
                        let lt = apply_subst_type(&s, &t1);
                        let rt = apply_subst_type(&s, &t2);
                        let target = select_float_type(&lt, &rt).ok_or_else(|| TypeError {
                            message: format!("Float op expects f32/f64, got {} and {}", lt, rt),
                            span: e.span.clone(),
                        })?;

                        let s3 = self.unify(&lt, &target).map_err(|m| TypeError {
                            message: m,
                            span: l.span.clone(),
                        })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self
                            .unify(&apply_subst_type(&s, &t2), &target)
                            .map_err(|m| TypeError {
                                message: m,
                                span: r.span.clone(),
                            })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, target))
                    }
                    BinaryOp::Eq | BinaryOp::Ne => {
                        let lt = apply_subst_type(&s, &t1);
                        let rt = apply_subst_type(&s, &t2);
                        // Eq/Ne work on int, char, string, and bool
                        let target = select_int_type(&lt, &rt)
                            .or_else(|| {
                                if matches!((&lt, &rt), (Type::Char, Type::Char)) {
                                    Some(Type::Char)
                                } else if matches!((&lt, &rt), (Type::String, Type::String)) {
                                    Some(Type::String)
                                } else if matches!((&lt, &rt), (Type::Bool, Type::Bool)) {
                                    Some(Type::Bool)
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| TypeError {
                                message: format!(
                                    "Equality comparison expects matching types, got {} and {}",
                                    lt, rt
                                ),
                                span: e.span.clone(),
                            })?;

                        let s3 = self.unify(&lt, &target).map_err(|m| TypeError {
                            message: m,
                            span: l.span.clone(),
                        })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self
                            .unify(&apply_subst_type(&s, &t2), &target)
                            .map_err(|m| TypeError {
                                message: m,
                                span: r.span.clone(),
                            })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Bool))
                    }
                    BinaryOp::Lt
                    | BinaryOp::Gt
                    | BinaryOp::Le
                    | BinaryOp::Ge => {
                        let lt = apply_subst_type(&s, &t1);
                        let rt = apply_subst_type(&s, &t2);
                        let target = select_int_type(&lt, &rt)
                            .or_else(|| {
                                if matches!((&lt, &rt), (Type::Char, Type::Char)) {
                                    Some(Type::Char)
                                } else {
                                    None
                                }
                            })
                            .ok_or_else(|| TypeError {
                            message: format!(
                                "Ordered comparison expects i32/i64/char, got {} and {}",
                                lt, rt
                            ),
                            span: e.span.clone(),
                        })?;

                        let s3 = self.unify(&lt, &target).map_err(|m| TypeError {
                            message: m,
                            span: l.span.clone(),
                        })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self
                            .unify(&apply_subst_type(&s, &t2), &target)
                            .map_err(|m| TypeError {
                                message: m,
                                span: r.span.clone(),
                            })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Bool))
                    }
                    BinaryOp::FEq
                    | BinaryOp::FNe
                    | BinaryOp::FLt
                    | BinaryOp::FGt
                    | BinaryOp::FLe
                    | BinaryOp::FGe => {
                        let lt = apply_subst_type(&s, &t1);
                        let rt = apply_subst_type(&s, &t2);
                        let target = select_float_type(&lt, &rt).ok_or_else(|| TypeError {
                            message: format!(
                                "Float comparison expects f32/f64, got {} and {}",
                                lt, rt
                            ),
                            span: e.span.clone(),
                        })?;

                        let s3 = self.unify(&lt, &target).map_err(|m| TypeError {
                            message: m,
                            span: l.span.clone(),
                        })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self
                            .unify(&apply_subst_type(&s, &t2), &target)
                            .map_err(|m| TypeError {
                                message: m,
                                span: r.span.clone(),
                            })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Bool))
                    }
                    BinaryOp::And | BinaryOp::Or => {
                        let s3 = self
                            .unify(&apply_subst_type(&s, &t1), &Type::Bool)
                            .map_err(|m| TypeError {
                                message: m,
                                span: l.span.clone(),
                            })?;
                        s = compose_subst(&s, &s3);
                        let s4 = self
                            .unify(&apply_subst_type(&s, &t2), &Type::Bool)
                            .map_err(|m| TypeError {
                                message: m,
                                span: r.span.clone(),
                            })?;
                        s = compose_subst(&s, &s4);
                        Ok((s, Type::Bool))
                    }
                }
            }
            Expr::Borrow(n, s) => {
                if let Some(sch) = env.get(&s.get_key(n)).cloned() {
                    let t = self.instantiate(&sch);
                    let i = match t {
                        Type::Linear(u) | Type::Borrow(u) => *u,
                        o => o,
                    };
                    Ok((HashMap::new(), Type::Borrow(Box::new(i))))
                } else {
                    Err(TypeError {
                        message: "Not found".into(),
                        span: e.span.clone(),
                    })
                }
            }
            Expr::Call { func, args } => {
                let (mut s, ft_raw) = if let Some(sch) = env.get(func).cloned() {
                    (HashMap::new(), self.instantiate(&sch))
                } else {
                    return Err(TypeError {
                        message: format!("Fn {} not found", func),
                        span: e.span.clone(),
                    });
                };
                let ft = match ft_raw {
                    Type::Linear(inner) => {
                        env.consume(func).map_err(|m| TypeError {
                            message: m,
                            span: e.span.clone(),
                        })?;
                        *inner
                    }
                    other => other,
                };
                // Explicit arity check before unification for better error messages
                if let Type::Arrow(ref expected_params, _, _, _) = ft {
                    if expected_params.len() != args.len() {
                        let expected_labels: Vec<&str> =
                            expected_params.iter().map(|(n, _)| n.as_str()).collect();
                        let provided_labels: Vec<&str> =
                            args.iter().map(|(n, _)| n.as_str()).collect();
                        return Err(TypeError {
                            message: format!(
                                "Arity mismatch in call to `{}`: expected {} arguments, got {}.\nExpected parameters: ({})\nProvided arguments: ({})",
                                func,
                                expected_params.len(),
                                args.len(),
                                expected_labels.join(", "),
                                provided_labels.join(", "),
                            ),
                            span: e.span.clone(),
                        });
                    }
                }
                let rt = self.new_var();
                let pts: Vec<(String, Type)> = args
                    .iter()
                    .map(|(n, _)| (n.clone(), self.new_var()))
                    .collect();
                let req = self.new_var();
                let ec = self.new_var();
                let sf = self
                    .unify(
                        &ft,
                        &Type::Arrow(
                            pts.clone(),
                            Box::new(rt.clone()),
                            Box::new(req.clone()),
                            Box::new(ec.clone()),
                        ),
                    )
                    .map_err(|m| TypeError {
                        message: m,
                        span: e.span.clone(),
                    })?;
                s = compose_subst(&s, &sf);
                let eci = apply_subst_type(&s, &ec);
                let eco = match eci {
                    Type::Row(el, None) => Type::Row(el, Some(Box::new(self.new_var()))),
                    Type::Unit => Type::Row(vec![], Some(Box::new(self.new_var()))),
                    o => o,
                };
                let se = self
                    .unify(&apply_subst_type(&s, ee), &eco)
                    .map_err(|m| TypeError {
                        message: m,
                        span: e.span.clone(),
                    })?;
                s = compose_subst(&s, &se);
                let reqi = apply_subst_type(&s, &req);
                let reqo = match reqi.clone() {
                    Type::Row(reqs, None) => Type::Row(reqs, Some(Box::new(self.new_var()))),
                    Type::Unit => Type::Row(vec![], Some(Box::new(self.new_var()))),
                    other => other,
                };
                let sr = self
                    .unify(&apply_subst_type(&s, eq), &reqo)
                    .map_err(|_| TypeError {
                        message: format!("Call to '{}' requires {}", func, reqi),
                        span: e.span.clone(),
                    })?;
                s = compose_subst(&s, &sr);

                for ((_, pt), (_, ae)) in pts.iter().zip(args) {
                    let (sa, ta) = self.infer(env, ae, er, eq, ee)?;
                    s = compose_subst(&s, &sa);
                    let expected = apply_subst_type(&s, pt);
                    let actual = apply_subst_type(&s, &ta);
                    let su = match self.unify(&actual, &expected) {
                        Ok(subst) => subst,
                        Err(primary_err) => {
                            // Linearity weakening at call sites:
                            // allow passing a plain value T to a linear parameter %T.
                            if let Type::Linear(inner) = expected {
                                if env.contains_linear_type(&actual) {
                                    return Err(TypeError {
                                        message: primary_err,
                                        span: ae.span.clone(),
                                    });
                                }
                                self.unify(&actual, &inner).map_err(|m| TypeError {
                                    message: format!(
                                        "{} (and linear weakening failed: {})",
                                        primary_err, m
                                    ),
                                    span: ae.span.clone(),
                                })?
                            } else {
                                return Err(TypeError {
                                    message: primary_err,
                                    span: ae.span.clone(),
                                });
                            }
                        }
                    };
                    s = compose_subst(&s, &su);
                }
                Ok((s.clone(), apply_subst_type(&s, &rt)))
            }
            Expr::Constructor(name, args) => {
                for ed in env.enums.values().cloned() {
                    if let Some(v) = ed.variants.iter().find(|x| x.name == *name) {
                        if v.fields.len() != args.len() {
                            return Err(TypeError {
                                message: format!(
                                    "Arity mismatch in constructor `{}`: expected {} arguments, got {}.\nExpected fields: {}\nProvided arguments: {}",
                                    name,
                                    v.fields.len(),
                                    args.len(),
                                    summarize_ctor_fields(&v.fields),
                                    summarize_ctor_args(args)
                                ),
                                span: e.span.clone(),
                            });
                        }
                        let mut s = HashMap::new();
                        let targs: Vec<Type> =
                            ed.type_params.iter().map(|_| self.new_var()).collect();
                        let mut inst = HashMap::new();
                        for (p, a) in ed.type_params.iter().zip(&targs) {
                            inst.insert(p.clone(), a.clone());
                        }
                        let mut matched = vec![None; v.fields.len()];
                        for (label, arg_expr) in args {
                            if let Some(l) = label {
                                if let Some(idx) =
                                    v.fields.iter().position(|(fl, _)| fl.as_ref() == Some(l))
                                {
                                    if matched[idx].is_some() {
                                        return Err(TypeError {
                                            message: format!(
                                                "Duplicate labeled argument `{}` in constructor `{}`.\nExpected fields: {}\nProvided arguments: {}",
                                                l,
                                                name,
                                                summarize_ctor_fields(&v.fields),
                                                summarize_ctor_args(args)
                                            ),
                                            span: arg_expr.span.clone(),
                                        });
                                    }
                                    matched[idx] = Some(arg_expr);
                                } else {
                                    return Err(TypeError {
                                        message: format!(
                                            "Unknown label `{}` for constructor `{}`.\nExpected fields: {}\nProvided arguments: {}",
                                            l,
                                            name,
                                            summarize_ctor_fields(&v.fields),
                                            summarize_ctor_args(args)
                                        ),
                                        span: arg_expr.span.clone(),
                                    });
                                }
                            } else {
                                if let Some(idx) = matched.iter().position(|m| m.is_none()) {
                                    matched[idx] = Some(arg_expr);
                                } else {
                                    return Err(TypeError {
                                        message: format!(
                                            "Too many positional arguments for constructor `{}`.\nExpected fields: {}\nProvided arguments: {}",
                                            name,
                                            summarize_ctor_fields(&v.fields),
                                            summarize_ctor_args(args)
                                        ),
                                        span: arg_expr.span.clone(),
                                    });
                                }
                            }
                        }

                        for (i, (field_label, ft)) in v.fields.iter().enumerate() {
                            let ae = matched[i].ok_or_else(|| TypeError {
                                message: format!(
                                    "Missing constructor argument for `{}` at {}.\nExpected fields: {}\nProvided arguments: {}\nHint: provide all fields exactly once (labels are recommended).",
                                    name,
                                    describe_ctor_field(field_label, i),
                                    summarize_ctor_fields(&v.fields),
                                    summarize_ctor_args(args)
                                ),
                                span: e.span.clone(),
                            })?;
                            let (sa, ta) = self.infer(env, ae, er, eq, ee)?;
                            s = compose_subst(&s, &sa);
                            let su = self
                                .unify(&ta, &apply_subst_type(&s, &apply_subst_type(&inst, ft)))
                                .map_err(|m| TypeError {
                                    message: format!(
                                        "Type mismatch in constructor `{}` at {}.\nDetails: {}",
                                        name,
                                        describe_ctor_field(field_label, i),
                                        m
                                    ),
                                    span: ae.span.clone(),
                                })?;
                            s = compose_subst(&s, &su);
                        }
                        return Ok((
                            s.clone(),
                            Type::UserDefined(
                                ed.name.clone(),
                                targs.iter().map(|a| apply_subst_type(&s, a)).collect(),
                            ),
                        ));
                    }
                }
                Err(TypeError {
                    message: format!("Unknown ctor {}", name),
                    span: e.span.clone(),
                })
            }
            Expr::Record(fs) => {
                let mut s = HashMap::new();
                let mut rfs = Vec::new();
                for (n, ex) in fs {
                    let (sa, ta) = self.infer(env, ex, er, eq, ee)?;
                    s = compose_subst(&s, &sa);
                    rfs.push((n.clone(), ta));
                }
                Ok((s, Type::Record(rfs)))
            }
            Expr::Array(exprs) => {
                let elem_type = self.new_var();
                let mut s = HashMap::new();
                for ex in exprs {
                    let (s_ex, t_ex) = self.infer(env, ex, er, eq, ee)?;
                    s = compose_subst(&s, &s_ex);
                    let s_unify = self
                        .unify(&t_ex, &apply_subst_type(&s, &elem_type))
                        .map_err(|m| TypeError {
                            message: m,
                            span: ex.span.clone(),
                        })?;
                    s = compose_subst(&s, &s_unify);
                }
                let final_elem_type = apply_subst_type(&s, &elem_type);
                if contains_ref(&final_elem_type) {
                    return Err(TypeError {
                        message: "Array cannot contain References".into(),
                        span: e.span.clone(),
                    });
                }
                Ok((s, Type::Array(Box::new(final_elem_type))))
            }
            Expr::List(exprs) => {
                let elem_type = self.new_var();
                let mut s = HashMap::new();
                for ex in exprs {
                    let (s_ex, t_ex) = self.infer(env, ex, er, eq, ee)?;
                    s = compose_subst(&s, &s_ex);
                    let s_unify = self
                        .unify(&t_ex, &apply_subst_type(&s, &elem_type))
                        .map_err(|m| TypeError {
                            message: m,
                            span: ex.span.clone(),
                        })?;
                    s = compose_subst(&s, &s_unify);
                }
                let final_elem_type = apply_subst_type(&s, &elem_type);
                Ok((s, Type::List(Box::new(final_elem_type))))
            }
            Expr::Index(arr, idx) => {
                let (s1, t_arr) = self.infer(env, arr, er, eq, ee)?;
                let (s2, t_idx) = self.infer(env, idx, er, eq, ee)?;
                let mut s = compose_subst(&s1, &s2);
                let s_idx = self.unify(&t_idx, &Type::I64).map_err(|m| TypeError {
                    message: m,
                    span: idx.span.clone(),
                })?;
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
                        _ => {
                            return Err(TypeError {
                                message: "Indexing non-array".into(),
                                span: arr.span.clone(),
                            })
                        }
                    },
                    _ => {
                        let et = self.new_var();
                        let su = self
                            .unify(&t_arr_unwrapped, &Type::Array(Box::new(et.clone())))
                            .map_err(|m| TypeError {
                                message: m,
                                span: arr.span.clone(),
                            })?;
                        s = compose_subst(&s, &su);
                        apply_subst_type(&s, &et)
                    }
                };

                if env.contains_linear_type(&elem_t) {
                    return Err(TypeError {
                        message: "Cannot move linear element out of array".into(),
                        span: e.span.clone(),
                    });
                }
                Ok((s, elem_t))
            }
            Expr::FieldAccess(rec, fnm) => {
                let (s1, tr) = self.infer(env, rec, er, eq, ee)?;
                let tr = apply_subst_type(&s1, &tr);
                if let Type::Record(fs) = &tr {
                    if let Some((_, t)) = fs.iter().find(|(n, _)| n == fnm) {
                        return Ok((s1, t.clone()));
                    }
                }
                if let Type::UserDefined(tn, ta) = &tr {
                    if let Some(td) = env.get_type(tn).cloned() {
                        if let Some((_, ft)) = td.fields.iter().find(|(n, _)| n == fnm) {
                            let mut su = HashMap::new();
                            for (p, a) in td.type_params.iter().zip(ta) {
                                su.insert(p.clone(), a.clone());
                            }
                            return Ok((s1, apply_subst_type(&su, ft)));
                        }
                    }
                }
                Err(TypeError {
                    message: format!("Field {} not found", fnm),
                    span: e.span.clone(),
                })
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let (s1, tc) = self.infer(env, cond, er, eq, ee)?;
                let s = compose_subst(
                    &s1,
                    &self.unify(&tc, &Type::Bool).map_err(|m| TypeError {
                        message: m,
                        span: cond.span.clone(),
                    })?,
                );
                let mut et = env.clone();
                et.apply(&s);
                self.infer_body(then_branch, &mut et, er, eq, ee)?;
                let mut ee_env = env.clone();
                ee_env.apply(&s);
                if let Some(eb) = else_branch {
                    self.infer_body(eb, &mut ee_env, er, eq, ee)?;
                }
                if et.linear_vars != ee_env.linear_vars {
                    return Err(TypeError {
                        message: "Linear mismatch".into(),
                        span: e.span.clone(),
                    });
                }
                env.linear_vars = et.linear_vars;
                Ok((s, Type::Unit))
            }
            Expr::Match { target, cases } => {
                let (s1, tt) = self.infer(env, target, er, eq, ee)?;
                let mut s = s1;
                self.check_exhaustiveness(env, &apply_subst_type(&s, &tt), cases)
                    .map_err(|m| TypeError {
                        message: m,
                        span: e.span.clone(),
                    })?;
                let mut rv: Option<HashSet<String>> = None;
                // None = diverges (return), Some(type) = tail expression type
                let mut case_tail_types: Vec<Option<Type>> = Vec::new();
                for case in cases {
                    let mut le = env.clone();
                    le.apply(&s);
                    let sm =
                        self.bind_pattern(&case.pattern, &apply_subst_type(&s, &tt), &mut le)?;
                    s = compose_subst(&s, &sm);
                    le.apply(&sm);
                    // Infer body and capture tail expression type
                    if case.body.is_empty() {
                        case_tail_types.push(Some(Type::Unit));
                    } else {
                        let last_idx = case.body.len() - 1;
                        self.infer_body(&case.body[..last_idx], &mut le, er, eq, ee)?;
                        let last = &case.body[last_idx];
                        match &last.node {
                            Stmt::Expr(expr) => {
                                let (s_tail, t_tail) = self.infer(&mut le, expr, er, eq, ee)?;
                                let tail = apply_subst_type(&s_tail, &t_tail);
                                s = compose_subst(&s, &s_tail);
                                let diverges = matches!(&expr.node, Expr::Raise(_));
                                if diverges {
                                    case_tail_types.push(None);
                                } else {
                                    case_tail_types.push(Some(apply_subst_type(&s, &tail)));
                                }
                            }
                            Stmt::Return(expr) => {
                                let (s1, t1) = self.infer(&mut le, expr, er, eq, ee)?;
                                le.apply(&s1);
                                le.check_unused_linear(&expr.span)?;
                                self.unify(&t1, &apply_subst_type(&s1, er)).map_err(|err| {
                                    TypeError {
                                        message: err,
                                        span: expr.span.clone(),
                                    }
                                })?;
                                case_tail_types.push(None); // diverges
                            }
                            _ => {
                                self.infer_body(&case.body[last_idx..], &mut le, er, eq, ee)?;
                                case_tail_types.push(Some(Type::Unit));
                            }
                        }
                    }
                    let case_diverges = case_tail_types.last() == Some(&None);
                    if !case_diverges {
                        if let Some(p) = &rv {
                            if p != &le.linear_vars {
                                return Err(TypeError {
                                    message: "Linear mismatch".into(),
                                    span: case.pattern.span.clone(),
                                });
                            }
                        } else {
                            rv = Some(le.linear_vars.clone());
                        }
                    }
                }
                if let Some(vars) = rv {
                    env.linear_vars = vars;
                } else {
                    // All cases diverge (return/raise) — code after match is
                    // unreachable, so clear linear obligations.
                    env.linear_vars.clear();
                }
                // Unify non-diverging case tail types
                let non_diverging: Vec<&Type> =
                    case_tail_types.iter().filter_map(|t| t.as_ref()).collect();
                if non_diverging.is_empty() {
                    Ok((s, Type::Unit))
                } else {
                    let mut result_type = non_diverging[0].clone();
                    for ct in &non_diverging[1..] {
                        let su = self.unify(&result_type, ct).map_err(|_| TypeError {
                            message:
                                "Match case type mismatch: all cases must produce the same type"
                                    .into(),
                            span: e.span.clone(),
                        })?;
                        s = compose_subst(&s, &su);
                        result_type = apply_subst_type(&su, &result_type);
                    }
                    Ok((s, result_type))
                }
            }
            Expr::While { cond, body } => {
                let (s1, tc) = self.infer(env, cond, er, eq, ee)?;
                let s = compose_subst(
                    &s1,
                    &self.unify(&tc, &Type::Bool).map_err(|m| TypeError {
                        message: m,
                        span: cond.span.clone(),
                    })?,
                );
                let mut le = env.clone();
                le.apply(&s);
                self.infer_body(body, &mut le, er, eq, ee)?;
                Ok((s, Type::Unit))
            }
            Expr::For {
                var,
                start,
                end_expr,
                body,
            } => {
                let (s1, ts) = self.infer(env, start, er, eq, ee)?;
                env.apply(&s1);
                let su_start = self.unify(&ts, &Type::I64).map_err(|m| TypeError {
                    message: m,
                    span: start.span.clone(),
                })?;
                let mut s = compose_subst(&s1, &su_start);
                let (s2, te) = self.infer(env, end_expr, er, eq, ee)?;
                s = compose_subst(&s, &s2);
                let su_end = self.unify(&te, &Type::I64).map_err(|m| TypeError {
                    message: m,
                    span: end_expr.span.clone(),
                })?;
                s = compose_subst(&s, &su_end);
                let mut le = env.clone();
                le.apply(&s);
                le.insert(
                    var.clone(),
                    Scheme {
                        vars: vec![],
                        typ: Type::I64,
                    },
                );
                self.infer_body(body, &mut le, er, eq, ee)?;
                Ok((s, Type::Unit))
            }
            Expr::Lambda {
                type_params,
                params,
                ret_type,
                requires,
                throws,
                body,
            } => {
                if contains_ref(ret_type) {
                    return Err(TypeError {
                        message: "Cannot return Ref".into(),
                        span: e.span.clone(),
                    });
                }

                let mut lambda_env = env.clone();
                let vars_set: HashSet<String> = type_params.iter().cloned().collect();

                let outer_keys: HashSet<String> = env.vars.keys().cloned().collect();
                let captured = collect_lambda_captures(body, params, &outer_keys);
                let mut captured_linear_keys = HashSet::new();
                for key in &captured {
                    if let Some(sch) = env.get(key) {
                        if contains_ref(&sch.typ) {
                            return Err(TypeError {
                                message: format!("Lambda cannot capture Ref value '{}'", key),
                                span: e.span.clone(),
                            });
                        }
                        if env.contains_linear_type(&sch.typ) {
                            captured_linear_keys.insert(key.clone());
                        }
                    }
                }
                let before_linear = lambda_env.linear_vars.clone();
                for p in params {
                    lambda_env.insert(
                        p.sigil.get_key(&p.name),
                        Scheme {
                            vars: vec![],
                            typ: p.typ.clone(),
                        },
                    );
                }

                self.infer_body(body, &mut lambda_env, ret_type, requires, throws)?;
                if !contains_return(body) && !matches!(ret_type, Type::Unit) {
                    return Err(TypeError {
                        message:
                            "Function body has no return statement; implicit return type is Unit"
                                .into(),
                        span: e.span.clone(),
                    });
                }
                let remaining_lambda_linear: HashSet<String> = lambda_env
                    .linear_vars
                    .difference(&before_linear)
                    .filter(|k| {
                        if let Some(sch) = lambda_env.vars.get(*k) {
                            !is_auto_droppable(&sch.typ)
                        } else {
                            true
                        }
                    })
                    .cloned()
                    .collect();
                if !remaining_lambda_linear.is_empty() {
                    return Err(TypeError {
                        message: format!("Unused linear in lambda: {:?}", remaining_lambda_linear),
                        span: e.span.clone(),
                    });
                }
                let consumed_outer_linear: HashSet<String> = before_linear
                    .difference(&lambda_env.linear_vars)
                    .cloned()
                    .collect();
                captured_linear_keys.extend(consumed_outer_linear);
                let has_linear_capture = !captured_linear_keys.is_empty();
                for key in captured_linear_keys {
                    env.consume(&key).map_err(|m| TypeError {
                        message: m,
                        span: e.span.clone(),
                    })?;
                }

                let arrow_typ = Type::Arrow(
                    params
                        .iter()
                        .map(|p| {
                            let t = self.convert_user_defined_to_var(&p.typ, &vars_set);
                            let t = if matches!(p.sigil, Sigil::Linear) {
                                Type::Linear(Box::new(t))
                            } else {
                                t
                            };
                            (p.name.clone(), t)
                        })
                        .collect(),
                    Box::new(self.convert_user_defined_to_var(ret_type, &vars_set)),
                    Box::new(self.convert_user_defined_to_var(requires, &vars_set)),
                    Box::new(self.convert_user_defined_to_var(throws, &vars_set)),
                );

                Ok((
                    HashMap::new(),
                    if has_linear_capture {
                        Type::Linear(Box::new(arrow_typ))
                    } else {
                        arrow_typ
                    },
                ))
            }
            Expr::External(_, _, typ) => Ok((HashMap::new(), typ.clone())),
            Expr::Handler {
                coeffect_name,
                requires: handler_requires,
                functions,
            } => {
                let prefix = format!("{}.", coeffect_name);
                let expected_methods: HashMap<String, Type> = env
                    .vars
                    .iter()
                    .filter_map(|(name, sch)| {
                        name.strip_prefix(&prefix)
                            .map(|method| (method.to_string(), self.instantiate(sch)))
                    })
                    .collect();

                let mut implemented = HashSet::new();
                for f in functions {
                    self.check_function(f, env, &e.span, handler_requires)?;

                    let Some(expected_method_type) = expected_methods.get(&f.name).cloned() else {
                        return Err(TypeError {
                            message: format!(
                                "Handler '{}.{}' is not declared in port '{}'",
                                coeffect_name, f.name, coeffect_name
                            ),
                            span: e.span.clone(),
                        });
                    };

                    let expected_impl_type =
                        strip_required_port_coeffect(&expected_method_type, coeffect_name.as_str());
                    let actual_impl_type = Type::Arrow(
                        f.params
                            .iter()
                            .map(|p| (p.name.clone(), p.typ.clone()))
                            .collect(),
                        Box::new(f.ret_type.clone()),
                        Box::new(f.requires.clone()),
                        Box::new(f.throws.clone()),
                    );
                    self.unify(&actual_impl_type, &expected_impl_type)
                        .map_err(|m| TypeError {
                            message: format!(
                                "Handler '{}.{}' signature mismatch: {}",
                                coeffect_name, f.name, m
                            ),
                            span: e.span.clone(),
                        })?;
                    implemented.insert(f.name.clone());
                }

                let mut missing: Vec<String> = expected_methods
                    .keys()
                    .filter(|method| !implemented.contains(*method))
                    .cloned()
                    .collect();
                missing.sort();
                if !missing.is_empty() {
                    return Err(TypeError {
                        message: format!(
                            "Handler '{}' is missing methods: {}",
                            coeffect_name,
                            missing.join(", ")
                        ),
                        span: e.span.clone(),
                    });
                }
                Ok((
                    HashMap::new(),
                    Type::Handler(coeffect_name.clone(), Box::new(handler_requires.clone())),
                ))
            }
            Expr::Raise(ex) => {
                let (s, t) = self.infer(env, ex, er, eq, ee)?;
                let exn_value_type = Type::UserDefined("Exn".into(), vec![]);
                let ss = self.unify(&t, &exn_value_type).map_err(|m| TypeError {
                    message: m,
                    span: ex.span.clone(),
                })?;
                let mut s = compose_subst(&s, &ss);
                let exn_type = Type::UserDefined("Exn".into(), vec![]);
                let required_eff = Type::Row(vec![exn_type], Some(Box::new(self.new_var())));
                let s_eff = self
                    .unify(&apply_subst_type(&s, ee), &required_eff)
                    .map_err(|_| TypeError {
                        message: "raise requires 'Exn'".into(),
                        span: e.span.clone(),
                    })?;
                s = compose_subst(&s, &s_eff);
                Ok((s, self.new_var()))
            }
        }
    }

    fn bind_pattern(
        &mut self,
        p: &Spanned<Pattern>,
        tt: &Type,
        env: &mut TypeEnv,
    ) -> Result<Subst, TypeError> {
        // Unwrap Linear/Borrow wrappers to get the structural type for pattern matching.
        let tt = match tt {
            Type::Linear(inner) | Type::Borrow(inner) => inner.as_ref(),
            other => other,
        };
        match &p.node {
            Pattern::Variable(n, sigil) => {
                env.insert(
                    sigil.get_key(n),
                    Scheme {
                        vars: vec![],
                        typ: tt.clone(),
                    },
                );
                Ok(HashMap::new())
            }
            Pattern::Constructor(n, pats) => {
                for ed in env.enums.values().cloned() {
                    if let Some(v) = ed.variants.iter().find(|x| x.name == *n) {
                        if v.fields.len() != pats.len() {
                            return Err(TypeError {
                                message: format!(
                                    "Arity mismatch in pattern `{}`: expected {} fields, got {}.\nExpected fields: {}\nProvided pattern arguments: {}",
                                    n,
                                    v.fields.len(),
                                    pats.len(),
                                    summarize_ctor_fields(&v.fields),
                                    summarize_ctor_args(pats)
                                ),
                                span: p.span.clone(),
                            });
                        }
                        let targs: Vec<Type> =
                            ed.type_params.iter().map(|_| self.new_var()).collect();
                        let s_en = self
                            .unify(tt, &Type::UserDefined(ed.name.clone(), targs.clone()))
                            .map_err(|m| TypeError {
                                message: m,
                                span: p.span.clone(),
                            })?;
                        let mut subst = s_en;
                        let mut inst = HashMap::new();
                        for (pa, a) in ed.type_params.iter().zip(targs) {
                            inst.insert(pa.clone(), a.clone());
                        }
                        let mut matched = vec![None; v.fields.len()];
                        for (label, pat) in pats {
                            if let Some(l) = label {
                                if let Some(idx) =
                                    v.fields.iter().position(|(fl, _)| fl.as_ref() == Some(l))
                                {
                                    if matched[idx].is_some() {
                                        return Err(TypeError {
                                            message: format!(
                                                "Duplicate labeled pattern argument `{}` in constructor `{}`.\nExpected fields: {}\nProvided pattern arguments: {}",
                                                l,
                                                n,
                                                summarize_ctor_fields(&v.fields),
                                                summarize_ctor_args(pats)
                                            ),
                                            span: pat.span.clone(),
                                        });
                                    }
                                    matched[idx] = Some(pat);
                                } else {
                                    return Err(TypeError {
                                        message: format!(
                                            "Unknown label `{}` for constructor pattern `{}`.\nExpected fields: {}\nProvided pattern arguments: {}",
                                            l,
                                            n,
                                            summarize_ctor_fields(&v.fields),
                                            summarize_ctor_args(pats)
                                        ),
                                        span: pat.span.clone(),
                                    });
                                }
                            } else {
                                if let Some(idx) = matched.iter().position(|m| m.is_none()) {
                                    matched[idx] = Some(pat);
                                } else {
                                    return Err(TypeError {
                                        message: format!(
                                            "Too many positional pattern arguments for constructor `{}`.\nExpected fields: {}\nProvided pattern arguments: {}",
                                            n,
                                            summarize_ctor_fields(&v.fields),
                                            summarize_ctor_args(pats)
                                        ),
                                        span: pat.span.clone(),
                                    });
                                }
                            }
                        }

                        for (i, (field_label, ft)) in v.fields.iter().enumerate() {
                            let pt = matched[i].ok_or_else(|| TypeError {
                                message: format!(
                                    "Missing constructor pattern argument for `{}` at {}.\nExpected fields: {}\nProvided pattern arguments: {}\nHint: provide a pattern for every field.",
                                    n,
                                    describe_ctor_field(field_label, i),
                                    summarize_ctor_fields(&v.fields),
                                    summarize_ctor_args(pats)
                                ),
                                span: p.span.clone(),
                            })?;
                            let sp = self.bind_pattern(
                                pt,
                                &apply_subst_type(&subst, &apply_subst_type(&inst, ft)),
                                env,
                            )?;
                            subst = compose_subst(&subst, &sp);
                        }
                        return Ok(subst);
                    }
                }
                Err(TypeError {
                    message: format!("Unknown ctor {}", n),
                    span: p.span.clone(),
                })
            }
            Pattern::Literal(l) => {
                let tl = match l {
                    Literal::Int(_) => Type::IntLit,
                    Literal::Float(_) => Type::FloatLit,
                    Literal::Bool(_) => Type::Bool,
                    Literal::Char(_) => Type::Char,
                    Literal::String(_) => Type::String,
                    Literal::Unit => Type::Unit,
                };
                self.unify(tt, &tl).map_err(|m| TypeError {
                    message: m,
                    span: p.span.clone(),
                })
            }
            Pattern::Wildcard => {
                if env.contains_linear_type(tt) && !is_auto_droppable(tt) {
                    return Err(TypeError {
                        message: format!(
                            "Wildcard pattern '_' cannot discard non-primitive linear value of type {:?}",
                            tt
                        ),
                        span: p.span.clone(),
                    });
                }
                Ok(HashMap::new())
            }
            Pattern::Record(pfs, open) => {
                let tfs = match tt {
                    Type::Record(fs) => {
                        let mut m = HashMap::new();
                        for (n, t) in fs {
                            m.insert(n.clone(), t.clone());
                        }
                        m
                    }
                    Type::UserDefined(n, args) => {
                        if let Some(td) = env.get_type(n).cloned() {
                            let mut m = HashMap::new();
                            let mut su = HashMap::new();
                            for (pa, a) in td.type_params.iter().zip(args) {
                                su.insert(pa.clone(), a.clone());
                            }
                            for (nm, t) in &td.fields {
                                m.insert(nm.clone(), apply_subst_type(&su, t));
                            }
                            m
                        } else {
                            return Err(TypeError {
                                message: "Unknown type".into(),
                                span: p.span.clone(),
                            });
                        }
                    }
                    _ => {
                        return Err(TypeError {
                            message: "Not record".into(),
                            span: p.span.clone(),
                        })
                    }
                };
                let mut sub = HashMap::new();
                let mut matched = HashSet::new();
                for (n, pt) in pfs {
                    if let Some(tf) = tfs.get(n) {
                        let sp = self.bind_pattern(pt, &apply_subst_type(&sub, tf), env)?;
                        sub = compose_subst(&sub, &sp);
                        matched.insert(n.clone());
                    } else {
                        return Err(TypeError {
                            message: format!("No field {}", n),
                            span: pt.span.clone(),
                        });
                    }
                }
                if !open {
                    for k in tfs.keys() {
                        if !matched.contains(k) {
                            return Err(TypeError {
                                message: format!("Missing {}", k),
                                span: p.span.clone(),
                            });
                        }
                    }
                }
                Ok(sub)
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
}
