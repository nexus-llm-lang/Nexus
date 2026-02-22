use super::ast::*;
use super::parser;
use crate::lang::stdlib::load_stdlib_nx_programs;
use chumsky::Parser;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

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

#[derive(Clone, Default)]
pub struct TypeEnv {
    pub vars: HashMap<String, Scheme>,
    pub types: HashMap<String, TypeDef>,
    pub enums: HashMap<String, EnumDef>,
    pub linear_vars: HashSet<String>,
    pub modules: HashMap<String, TypeEnv>,
}

type Subst = HashMap<String, Type>;

impl TypeEnv {
    /// Creates an empty typing environment.
    pub fn new() -> Self {
        TypeEnv {
            vars: HashMap::new(),
            types: HashMap::new(),
            enums: HashMap::new(),
            linear_vars: HashSet::new(),
            modules: HashMap::new(),
        }
    }

    /// Returns whether a type contains linear data anywhere inside it.
    pub fn contains_linear_type(&self, typ: &Type) -> bool {
        let mut visiting = HashSet::new();
        self.contains_linear_type_inner(typ, &mut visiting)
    }

    fn contains_linear_type_inner(&self, typ: &Type, visiting: &mut HashSet<String>) -> bool {
        match typ {
            Type::Linear(_) | Type::Array(_) => true,
            Type::Borrow(_) => false,
            Type::Ref(inner) => self.contains_linear_type_inner(inner, visiting),
            Type::Arrow(_, _, _) => false,
            Type::UserDefined(name, args) => {
                if args
                    .iter()
                    .any(|arg| self.contains_linear_type_inner(arg, visiting))
                {
                    return true;
                }

                if !visiting.insert(name.clone()) {
                    return false;
                }

                let mut subst = HashMap::new();
                let mut has_linear = false;

                if let Some(td) = self.get_type(name) {
                    for (param, arg) in td.type_params.iter().zip(args.iter()) {
                        subst.insert(param.clone(), arg.clone());
                    }
                    has_linear = td.fields.iter().any(|(_, field_type)| {
                        let instantiated = apply_subst_type(&subst, field_type);
                        self.contains_linear_type_inner(&instantiated, visiting)
                    });
                } else if let Some(ed) = self.get_enum(name) {
                    for (param, arg) in ed.type_params.iter().zip(args.iter()) {
                        subst.insert(param.clone(), arg.clone());
                    }
                    has_linear = ed.variants.iter().any(|variant| {
                        variant.fields.iter().any(|(_, field_type)| {
                            let instantiated = apply_subst_type(&subst, field_type);
                            self.contains_linear_type_inner(&instantiated, visiting)
                        })
                    });
                }

                visiting.remove(name);
                has_linear
            }
            Type::Row(effs, tail) => {
                effs.iter()
                    .any(|eff| self.contains_linear_type_inner(eff, visiting))
                    || tail.as_ref().map_or(false, |tail| {
                        self.contains_linear_type_inner(tail, visiting)
                    })
            }
            Type::Record(fields) => fields
                .iter()
                .any(|(_, field_type)| self.contains_linear_type_inner(field_type, visiting)),
            _ => false,
        }
    }

    /// Inserts a variable scheme and tracks linear consumption state.
    pub fn insert(&mut self, name: String, scheme: Scheme) {
        if self.contains_linear_type(&scheme.typ) {
            self.linear_vars.insert(name.clone());
        }
        self.vars.insert(name, scheme);
    }

    /// Resolves a value binding by local name or `module.item`.
    pub fn get(&self, name: &str) -> Option<&Scheme> {
        if let Some(scheme) = self.vars.get(name) {
            return Some(scheme);
        }
        if let Some(pos) = name.find('.') {
            let mod_name = &name[..pos];
            let item_name = &name[pos + 1..];
            return self.modules.get(mod_name).and_then(|m| m.get(item_name));
        }
        None
    }

    /// Resolves a type definition by local name or `module.item`.
    pub fn get_type(&self, name: &str) -> Option<&TypeDef> {
        if let Some(pos) = name.find('.') {
            let mod_name = &name[..pos];
            let item_name = &name[pos + 1..];
            self.modules
                .get(mod_name)
                .and_then(|m| m.get_type(item_name))
        } else {
            self.types.get(name)
        }
    }

    /// Resolves an enum definition by local name or `module.item`.
    pub fn get_enum(&self, name: &str) -> Option<&EnumDef> {
        if let Some(pos) = name.find('.') {
            let mod_name = &name[..pos];
            let item_name = &name[pos + 1..];
            self.modules
                .get(mod_name)
                .and_then(|m| m.get_enum(item_name))
        } else {
            self.enums.get(name)
        }
    }

    /// Applies a substitution to all variable schemes in this environment.
    pub fn apply(&mut self, subst: &Subst) {
        for scheme in self.vars.values_mut() {
            scheme.typ = apply_subst_type(subst, &scheme.typ);
        }
        for module in self.modules.values_mut() {
            module.apply(subst);
        }
    }

    /// Marks a linear binding as consumed, returning an error on invalid reuse.
    pub fn consume(&mut self, name: &str) -> Result<(), String> {
        if self.linear_vars.remove(name) {
            Ok(())
        } else if self.vars.contains_key(name) {
            if let Some(s) = self.vars.get(name) {
                if self.contains_linear_type(&s.typ) {
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
    pub visited_paths: HashSet<String>,
}

fn get_default_alias(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

fn describe_ctor_field(label: &Option<String>, index: usize) -> String {
    match label {
        Some(name) => format!("#{} label `{}`", index + 1, name),
        None => format!("#{} positional field", index + 1),
    }
}

fn summarize_ctor_args<T>(args: &[(Option<String>, T)]) -> String {
    if args.is_empty() {
        return "none".to_string();
    }
    args.iter()
        .enumerate()
        .map(|(i, (label, _))| match label {
            Some(name) => format!("#{} label `{}`", i + 1, name),
            None => format!("#{} positional", i + 1),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn summarize_ctor_fields(fields: &[(Option<String>, Type)]) -> String {
    if fields.is_empty() {
        return "none".to_string();
    }
    fields
        .iter()
        .enumerate()
        .map(|(i, (label, _))| describe_ctor_field(label, i))
        .collect::<Vec<_>>()
        .join(", ")
}

fn exn_enum_def() -> EnumDef {
    EnumDef {
        name: "Exn".to_string(),
        is_public: true,
        type_params: vec![],
        variants: vec![
            VariantDef {
                name: "RuntimeError".to_string(),
                fields: vec![(Some("val".to_string()), Type::String)],
            },
            VariantDef {
                name: "InvalidIndex".to_string(),
                fields: vec![(Some("val".to_string()), Type::I64)],
            },
        ],
    }
}

fn register_nullary_variant_constructor(
    env: &mut TypeEnv,
    enum_name: &str,
    type_params: &[String],
    variant: &VariantDef,
) {
    if variant.fields.is_empty() {
        let targs = type_params
            .iter()
            .map(|p| Type::Var(p.clone()))
            .collect::<Vec<_>>();
        env.insert(
            variant.name.clone(),
            Scheme {
                vars: type_params.to_vec(),
                typ: Type::UserDefined(enum_name.to_string(), targs),
            },
        );
    }
}

fn register_exception_variant(
    env: &mut TypeEnv,
    exception: &ExceptionDef,
    span: &Span,
) -> Result<(), TypeError> {
    {
        let exn = env
            .enums
            .entry("Exn".to_string())
            .or_insert_with(exn_enum_def);
        if exn.variants.iter().any(|v| v.name == exception.name) {
            return Err(TypeError {
                message: format!("Duplicate exception constructor: {}", exception.name),
                span: span.clone(),
            });
        }
    }

    let variant = VariantDef {
        name: exception.name.clone(),
        fields: exception.fields.clone(),
    };
    register_nullary_variant_constructor(env, "Exn", &[], &variant);
    env.enums
        .entry("Exn".to_string())
        .or_insert_with(exn_enum_def)
        .variants
        .push(variant);
    Ok(())
}

fn convert_generic_user_defined_to_var(typ: &Type, vars: &HashSet<String>) -> Type {
    match typ {
        Type::UserDefined(n, args) => {
            if args.is_empty() && vars.contains(n) {
                Type::Var(n.clone())
            } else {
                Type::UserDefined(
                    n.clone(),
                    args.iter()
                        .map(|a| convert_generic_user_defined_to_var(a, vars))
                        .collect(),
                )
            }
        }
        Type::Arrow(p, r, e) => Type::Arrow(
            p.iter()
                .map(|(n, t)| (n.clone(), convert_generic_user_defined_to_var(t, vars)))
                .collect(),
            Box::new(convert_generic_user_defined_to_var(r, vars)),
            Box::new(convert_generic_user_defined_to_var(e, vars)),
        ),
        Type::Ref(i) => Type::Ref(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Linear(i) => Type::Linear(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Borrow(i) => Type::Borrow(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Array(i) => Type::Array(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Row(es, t) => Type::Row(
            es.iter()
                .map(|x| convert_generic_user_defined_to_var(x, vars))
                .collect(),
            t.as_ref()
                .map(|x| Box::new(convert_generic_user_defined_to_var(x, vars))),
        ),
        Type::Record(fs) => Type::Record(
            fs.iter()
                .map(|(n, t)| (n.clone(), convert_generic_user_defined_to_var(t, vars)))
                .collect(),
        ),
        _ => typ.clone(),
    }
}

fn normalize_typedef_generic_params(td: &TypeDef) -> TypeDef {
    let vars: HashSet<String> = td.type_params.iter().cloned().collect();
    TypeDef {
        name: td.name.clone(),
        is_public: td.is_public,
        type_params: td.type_params.clone(),
        fields: td
            .fields
            .iter()
            .map(|(n, t)| (n.clone(), convert_generic_user_defined_to_var(t, &vars)))
            .collect(),
    }
}

fn normalize_enum_generic_params(ed: &EnumDef) -> EnumDef {
    let vars: HashSet<String> = ed.type_params.iter().cloned().collect();
    EnumDef {
        name: ed.name.clone(),
        is_public: ed.is_public,
        type_params: ed.type_params.clone(),
        variants: ed
            .variants
            .iter()
            .map(|v| VariantDef {
                name: v.name.clone(),
                fields: v
                    .fields
                    .iter()
                    .map(|(label, t)| {
                        (label.clone(), convert_generic_user_defined_to_var(t, &vars))
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn default_numeric_literals(typ: &Type) -> Type {
    match typ {
        Type::IntLit => Type::I64,
        Type::FloatLit => Type::F64,
        Type::Arrow(params, ret, eff) => Type::Arrow(
            params
                .iter()
                .map(|(name, t)| (name.clone(), default_numeric_literals(t)))
                .collect(),
            Box::new(default_numeric_literals(ret)),
            Box::new(default_numeric_literals(eff)),
        ),
        Type::UserDefined(name, args) => Type::UserDefined(
            name.clone(),
            args.iter().map(default_numeric_literals).collect(),
        ),
        Type::Ref(inner) => Type::Ref(Box::new(default_numeric_literals(inner))),
        Type::Linear(inner) => Type::Linear(Box::new(default_numeric_literals(inner))),
        Type::Borrow(inner) => Type::Borrow(Box::new(default_numeric_literals(inner))),
        Type::Array(inner) => Type::Array(Box::new(default_numeric_literals(inner))),
        Type::Row(effs, tail) => Type::Row(
            effs.iter().map(default_numeric_literals).collect(),
            tail.as_ref().map(|t| Box::new(default_numeric_literals(t))),
        ),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(name, t)| (name.clone(), default_numeric_literals(t)))
                .collect(),
        ),
        _ => typ.clone(),
    }
}

fn select_int_type(left: &Type, right: &Type) -> Option<Type> {
    if matches!(left, Type::I32) || matches!(right, Type::I32) {
        return Some(Type::I32);
    }
    if matches!(left, Type::I64) || matches!(right, Type::I64) {
        return Some(Type::I64);
    }
    if matches!(left, Type::IntLit | Type::Var(_)) && matches!(right, Type::IntLit | Type::Var(_)) {
        return Some(Type::I64);
    }
    None
}

fn select_float_type(left: &Type, right: &Type) -> Option<Type> {
    if matches!(left, Type::F32) || matches!(right, Type::F32) {
        return Some(Type::F32);
    }
    if matches!(left, Type::F64) || matches!(right, Type::F64) {
        return Some(Type::F64);
    }
    if matches!(left, Type::FloatLit | Type::Var(_))
        && matches!(right, Type::FloatLit | Type::Var(_))
    {
        return Some(Type::F64);
    }
    None
}

fn collect_external_type_vars(typ: &Type, env: &TypeEnv, out: &mut HashSet<String>) {
    match typ {
        Type::UserDefined(name, args) => {
            for arg in args {
                collect_external_type_vars(arg, env, out);
            }
            if args.is_empty() && !env.types.contains_key(name) && !env.enums.contains_key(name) {
                out.insert(name.clone());
            }
        }
        Type::Arrow(params, ret, effects) => {
            for (_, typ) in params {
                collect_external_type_vars(typ, env, out);
            }
            collect_external_type_vars(ret, env, out);
            collect_external_type_vars(effects, env, out);
        }
        Type::Ref(inner) | Type::Linear(inner) | Type::Borrow(inner) | Type::Array(inner) => {
            collect_external_type_vars(inner, env, out)
        }
        Type::Row(effects, tail) => {
            for effect in effects {
                collect_external_type_vars(effect, env, out);
            }
            if let Some(tail) = tail {
                collect_external_type_vars(tail, env, out);
            }
        }
        Type::Record(fields) => {
            for (_, typ) in fields {
                collect_external_type_vars(typ, env, out);
            }
        }
        _ => {}
    }
}

fn convert_external_type_vars(typ: &Type, vars: &HashSet<String>) -> Type {
    match typ {
        Type::UserDefined(name, args) => {
            if args.is_empty() && vars.contains(name) {
                Type::Var(name.clone())
            } else {
                Type::UserDefined(
                    name.clone(),
                    args.iter()
                        .map(|arg| convert_external_type_vars(arg, vars))
                        .collect(),
                )
            }
        }
        Type::Arrow(params, ret, effects) => Type::Arrow(
            params
                .iter()
                .map(|(name, typ)| (name.clone(), convert_external_type_vars(typ, vars)))
                .collect(),
            Box::new(convert_external_type_vars(ret, vars)),
            Box::new(convert_external_type_vars(effects, vars)),
        ),
        Type::Ref(inner) => Type::Ref(Box::new(convert_external_type_vars(inner, vars))),
        Type::Linear(inner) => Type::Linear(Box::new(convert_external_type_vars(inner, vars))),
        Type::Borrow(inner) => Type::Borrow(Box::new(convert_external_type_vars(inner, vars))),
        Type::Array(inner) => Type::Array(Box::new(convert_external_type_vars(inner, vars))),
        Type::Row(effects, tail) => Type::Row(
            effects
                .iter()
                .map(|effect| convert_external_type_vars(effect, vars))
                .collect(),
            tail.as_ref()
                .map(|tail| Box::new(convert_external_type_vars(tail, vars))),
        ),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(name, typ)| (name.clone(), convert_external_type_vars(typ, vars)))
                .collect(),
        ),
        _ => typ.clone(),
    }
}

fn external_scheme(typ: &Type, env: &TypeEnv) -> Scheme {
    let mut vars = HashSet::new();
    collect_external_type_vars(typ, env, &mut vars);
    let mut vars_vec: Vec<String> = vars.into_iter().collect();
    vars_vec.sort();
    let vars_set: HashSet<String> = vars_vec.iter().cloned().collect();
    Scheme {
        typ: convert_external_type_vars(typ, &vars_set),
        vars: vars_vec,
    }
}

fn register_public_stdlib_from_nx(env: &mut TypeEnv) {
    let Ok(programs) = load_stdlib_nx_programs() else {
        return;
    };

    let checker = TypeChecker::new_without_stdlib();

    for (_path, program) in programs {
        // First pass: register types and signatures
        for def in &program.definitions {
            match &def.node {
                TopLevel::TypeDef(td) if td.is_public => {
                    let td_norm = normalize_typedef_generic_params(td);
                    env.types.insert(td_norm.name.clone(), td_norm);
                }
                TopLevel::Enum(ed) if ed.is_public => {
                    let ed_norm = normalize_enum_generic_params(ed);
                    env.enums.insert(ed_norm.name.clone(), ed_norm.clone());
                    for v in &ed_norm.variants {
                        register_nullary_variant_constructor(
                            env,
                            &ed_norm.name,
                            &ed_norm.type_params,
                            v,
                        );
                    }
                }
                TopLevel::Exception(ex) if ex.is_public => {
                    let _ = register_exception_variant(env, ex, &def.span);
                }
                TopLevel::Let(gl) if gl.is_public => match &gl.value.node {
                    Expr::Lambda {
                        type_params,
                        params,
                        ret_type,
                        effects,
                        ..
                    } => {
                        let vars_set: HashSet<String> = type_params.iter().cloned().collect();
                        env.insert(
                            gl.name.clone(),
                            Scheme {
                                vars: type_params.clone(),
                                typ: Type::Arrow(
                                    params
                                        .iter()
                                        .map(|p| {
                                            (
                                                p.name.clone(),
                                                checker
                                                    .convert_user_defined_to_var(&p.typ, &vars_set),
                                            )
                                        })
                                        .collect(),
                                    Box::new(
                                        checker.convert_user_defined_to_var(ret_type, &vars_set),
                                    ),
                                    Box::new(
                                        checker.convert_user_defined_to_var(effects, &vars_set),
                                    ),
                                ),
                            },
                        );
                    }
                    Expr::External(_, typ) => {
                        env.insert(gl.name.clone(), external_scheme(typ, env));
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}

impl TypeChecker {
    /// Creates a checker with only language-core builtins (no stdlib `.nx` imports).
    pub fn new_without_stdlib() -> Self {
        let mut env = TypeEnv::new();
        // Core effect markers that language/runtime rely on.
        env.types.insert(
            "IO".to_string(),
            TypeDef {
                name: "IO".to_string(),
                is_public: true,
                type_params: vec![],
                fields: vec![],
            },
        );
        env.enums.insert("Exn".to_string(), exn_enum_def());

        env.linear_vars.clear();
        TypeChecker {
            supply: 0,
            env,
            visited_paths: HashSet::new(),
        }
    }

    /// Creates a checker and registers public stdlib bindings.
    pub fn new() -> Self {
        let mut checker = Self::new_without_stdlib();
        register_public_stdlib_from_nx(&mut checker.env);
        checker
    }

    fn new_var(&mut self) -> Type {
        let n = self.supply;
        self.supply += 1;
        Type::Var(format!("?{}", n))
    }

    /// Type-checks a full program and updates internal environment state.
    pub fn check_program(&mut self, program: &Program) -> Result<(), TypeError> {
        // Pass 1: Collect imports, types, enums, exceptions, ports, and signatures of global lets
        for def in &program.definitions {
            match &def.node {
                TopLevel::Import(import) => {
                    if !import.is_external {
                        if self.visited_paths.contains(&import.path) {
                            continue;
                        }
                        self.visited_paths.insert(import.path.clone());

                        let src = fs::read_to_string(&import.path).map_err(|e| TypeError {
                            message: format!("Failed to read {}: {}", import.path, e),
                            span: def.span.clone(),
                        })?;
                        let p = parser::parser().parse(src).map_err(|_| TypeError {
                            message: format!("Failed to parse {}", import.path),
                            span: def.span.clone(),
                        })?;

                        let mut sub_checker = TypeChecker::new();
                        sub_checker.visited_paths = self.visited_paths.clone();
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
                                TopLevel::Exception(ex) if ex.is_public => {
                                    register_exception_variant(&mut public_env, ex, &sub_def.span)?;
                                }
                                TopLevel::Let(gl) if gl.is_public => {
                                    if let Some(sch) = sub_checker.env.vars.get(&gl.name) {
                                        public_env.insert(gl.name.clone(), sch.clone());
                                    }
                                }
                                _ => {}
                            }
                        }
                        self.visited_paths = sub_checker.visited_paths;

                        if !import.items.is_empty() {
                            for item in &import.items {
                                if let Some(sch) = public_env.get(item) {
                                    self.env.insert(item.clone(), sch.clone());
                                } else {
                                    return Err(TypeError {
                                        message: format!(
                                            "Item {} not found in {}",
                                            item, import.path
                                        ),
                                        span: def.span.clone(),
                                    });
                                }
                            }
                        } else {
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
                        self.env.insert(
                            name,
                            Scheme {
                                vars: vec![],
                                typ: Type::Arrow(
                                    ptypes,
                                    Box::new(sig.ret_type.clone()),
                                    Box::new(sig.effects.clone()),
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
                            effects,
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
                                            self.convert_user_defined_to_var(effects, &vars_set),
                                        ),
                                    ),
                                },
                            );
                        }
                        Expr::External(_, typ) => {
                            let scheme = external_scheme(typ, &self.env);
                            self.env.insert(gl.name.clone(), scheme);
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        // Pass 2: Check all global let bodies and handlers
        self.env.linear_vars.clear();
        for def in &program.definitions {
            match &def.node {
                TopLevel::Let(gl) => {
                    let v = self.new_var();
                    let mut env = std::mem::take(&mut self.env);
                    let res = self.infer(&mut env, &gl.value, &Type::Unit, &v);
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

                    if gl.name == "main" {
                        if gl.is_public {
                            return Err(TypeError {
                                message: "main function must be private (remove 'pub')".into(),
                                span: def.span.clone(),
                            });
                        }
                        let rt = self.new_var();
                        let ef = self.new_var();
                        let sm = self
                            .unify(
                                &t,
                                &Type::Arrow(vec![], Box::new(rt.clone()), Box::new(ef.clone())),
                            )
                            .map_err(|_| TypeError {
                                message: "main must be a function '() -> T'".into(),
                                span: def.span.clone(),
                            })?;
                        let final_ef = apply_subst_type(&sm, &ef);
                        if contains_exn_effect(&final_ef) {
                            return Err(TypeError {
                                message: "main function must not declare Exn effect".into(),
                                span: def.span.clone(),
                            });
                        }
                        if !is_allowed_main_effect_signature(&final_ef) {
                            return Err(TypeError {
                                message:
                                    "main function effects must be one of: {}, { IO }, { IO, Net }"
                                        .into(),
                                span: def.span.clone(),
                            });
                        }
                    }

                    self.env
                        .insert(gl.name.clone(), self.generalize(&self.env, t));
                }
                TopLevel::Handler(h) => {
                    self.check_handler(h, &def.span)?;
                }
                _ => {}
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
            Type::Arrow(p, r, e) => Type::Arrow(
                p.iter()
                    .map(|(n, t)| (n.clone(), self.convert_user_defined_to_var(t, vars)))
                    .collect(),
                Box::new(self.convert_user_defined_to_var(r, vars)),
                Box::new(self.convert_user_defined_to_var(e, vars)),
            ),
            Type::Ref(i) => Type::Ref(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Linear(i) => Type::Linear(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Borrow(i) => Type::Borrow(Box::new(self.convert_user_defined_to_var(i, vars))),
            Type::Array(i) => Type::Array(Box::new(self.convert_user_defined_to_var(i, vars))),
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

    fn check_function(&mut self, func: &Function, span: &Span) -> Result<(), TypeError> {
        let mut env = self.env.clone();
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
        self.infer_body(&func.body, &mut env, &func.ret_type, &func.effects)?;
        if !env.linear_vars.is_empty() {
            return Err(TypeError {
                message: format!("Unused linear: {:?}", env.linear_vars),
                span: span.clone(),
            });
        }
        Ok(())
    }

    fn check_handler(&mut self, h: &Handler, span: &Span) -> Result<(), TypeError> {
        for f in &h.functions {
            let name = format!("{}.{}", h.port_name, f.name);
            if let Some(sch) = self.env.get(&name).cloned() {
                self.unify(&sch.typ, &self.generalize_top_level(f).typ)
                    .map_err(|e| TypeError {
                        message: e,
                        span: span.clone(),
                    })?;
                self.check_function(f, span)?;
            } else {
                return Err(TypeError {
                    message: format!("Fn {} not in port", f.name),
                    span: span.clone(),
                });
            }
        }
        Ok(())
    }

    fn infer_body(
        &mut self,
        body: &[Spanned<Stmt>],
        env: &mut TypeEnv,
        er: &Type,
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
                        let ann = typ.clone().ok_or_else(|| TypeError {
                            message: "Recursive lambda requires an explicit type annotation".into(),
                            span: value.span.clone(),
                        })?;
                        env.insert(
                            key.clone(),
                            Scheme {
                                vars: vec![],
                                typ: ann,
                            },
                        );
                    }
                    let (s1, t1) = self.infer(env, value, er, ee)?;
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
                            if env.contains_linear_type(&t1) {
                                t1
                            } else {
                                Type::Linear(Box::new(t1))
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
                    };
                    env.insert(key, self.generalize(env, ft));
                }
                Stmt::Return(e) => {
                    let (s1, t1) = self.infer(env, e, er, ee)?;
                    env.apply(&s1);
                    if !env.linear_vars.is_empty() {
                        return Err(TypeError {
                            message: "Unused linear".into(),
                            span: e.span.clone(),
                        });
                    }
                    self.unify(&t1, &apply_subst_type(&s1, er))
                        .map_err(|err| TypeError {
                            message: err,
                            span: e.span.clone(),
                        })?;
                }
                Stmt::Drop(e) => {
                    self.infer(env, e, er, ee)?;
                }
                Stmt::Expr(e) => {
                    self.infer(env, e, er, ee)?;
                }
                Stmt::Assign { target, value } => {
                    let (s_v, t_v) = self.infer(env, value, er, ee)?;
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
                            let (s_idx, t_idx) = self.infer(env, idx, er, ee)?;
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
                                    let (s_a, t_a) = self.infer(env, arr, er, ee)?;
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
                        self.check_task(t, env, &s.span)?;
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
                    self.infer_body(body, &mut et, er, &try_eff)?;
                    let mut ec = env.clone();
                    ec.insert(
                        catch_param.clone(),
                        Scheme {
                            vars: vec![],
                            typ: Type::UserDefined("Exn".into(), vec![]),
                        },
                    );
                    self.infer_body(catch_body, &mut ec, er, ee)?;
                    if et.linear_vars != ec.linear_vars {
                        return Err(TypeError {
                            message: "Linear mismatch".into(),
                            span: s.span.clone(),
                        });
                    }
                    env.linear_vars = et.linear_vars;
                }
                Stmt::Comment => {}
            }
        }
        Ok(())
    }

    fn check_task(&mut self, t: &Function, oe: &TypeEnv, _span: &Span) -> Result<(), TypeError> {
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
        self.infer_body(&t.body, &mut te, &Type::Unit, &t.effects)?;

        let unused_local_linear: Vec<_> = te
            .linear_vars
            .iter()
            .filter(|k| !captured_linear.contains(*k))
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
            match &s.node {
                Stmt::Expr(e) => {
                    let (sub, t) = self.infer(&mut env, e, &Type::Unit, &ev)?;
                    env.apply(&sub);
                    Ok(default_numeric_literals(&apply_subst_type(&sub, &t)))
                }
                _ => {
                    self.infer_body(&[s.clone()], &mut env, &Type::Unit, &ev)?;
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
        ee: &Type,
    ) -> Result<(Subst, Type), TypeError> {
        match &e.node {
            Expr::Literal(l) => Ok((
                HashMap::new(),
                match l {
                    Literal::Int(_) => Type::IntLit,
                    Literal::Float(_) => Type::FloatLit,
                    Literal::Bool(_) => Type::Bool,
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
                let (s1, t1) = self.infer(env, l, er, ee)?;
                let (s2, t2) = self.infer(env, r, er, ee)?;
                let mut s = compose_subst(&s1, &s2);
                match op.as_str() {
                    "+" | "-" | "*" | "/" => {
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
                    "++" => {
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
                    "+." | "-." | "*." | "/." => {
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
                    "==" | "!=" | "<" | ">" | "<=" | ">=" => {
                        let lt = apply_subst_type(&s, &t1);
                        let rt = apply_subst_type(&s, &t2);
                        let target = select_int_type(&lt, &rt).ok_or_else(|| TypeError {
                            message: format!(
                                "Integer comparison expects i32/i64, got {} and {}",
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
                    "==." | "!=." | "<." | ">." | "<=." | ">=." => {
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
                    _ => Err(TypeError {
                        message: format!("Unknown op {}", op),
                        span: e.span.clone(),
                    }),
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
            Expr::Call {
                func,
                args,
                perform,
            } => {
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
                let rt = self.new_var();
                let pts: Vec<(String, Type)> = args
                    .iter()
                    .map(|(n, _)| (n.clone(), self.new_var()))
                    .collect();
                let ec = self.new_var();
                let sf = self
                    .unify(
                        &ft,
                        &Type::Arrow(pts.clone(), Box::new(rt.clone()), Box::new(ec.clone())),
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

                // Enforce perform
                let actual_eff = apply_subst_type(&s, &ec);
                let is_pure = match actual_eff {
                    Type::Row(effs, tail) => effs.is_empty() && tail.is_none(),
                    Type::Unit => true,
                    _ => false,
                };
                if !perform && !is_pure {
                    return Err(TypeError {
                        message: "Effectful call requires 'perform'".into(),
                        span: e.span.clone(),
                    });
                }
                if *perform && is_pure {
                    return Err(TypeError {
                        message: "Pure call should not use 'perform'".into(),
                        span: e.span.clone(),
                    });
                }

                for ((_, pt), (_, ae)) in pts.iter().zip(args) {
                    let (sa, ta) = self.infer(env, ae, er, ee)?;
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
                            let (sa, ta) = self.infer(env, ae, er, ee)?;
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
                    let (sa, ta) = self.infer(env, ex, er, ee)?;
                    s = compose_subst(&s, &sa);
                    rfs.push((n.clone(), ta));
                }
                Ok((s, Type::Record(rfs)))
            }
            Expr::Array(exprs) => {
                let elem_type = self.new_var();
                let mut s = HashMap::new();
                for ex in exprs {
                    let (s_ex, t_ex) = self.infer(env, ex, er, ee)?;
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
            Expr::Index(arr, idx) => {
                let (s1, t_arr) = self.infer(env, arr, er, ee)?;
                let (s2, t_idx) = self.infer(env, idx, er, ee)?;
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
                let (s1, tr) = self.infer(env, rec, er, ee)?;
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
                let (s1, tc) = self.infer(env, cond, er, ee)?;
                let s = compose_subst(
                    &s1,
                    &self.unify(&tc, &Type::Bool).map_err(|m| TypeError {
                        message: m,
                        span: cond.span.clone(),
                    })?,
                );
                let mut et = env.clone();
                et.apply(&s);
                self.infer_body(then_branch, &mut et, er, ee)?;
                let mut ee_env = env.clone();
                ee_env.apply(&s);
                if let Some(eb) = else_branch {
                    self.infer_body(eb, &mut ee_env, er, ee)?;
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
                let (s1, tt) = self.infer(env, target, er, ee)?;
                let mut s = s1;
                self.check_exhaustiveness(env, &apply_subst_type(&s, &tt), cases)
                    .map_err(|m| TypeError {
                        message: m,
                        span: e.span.clone(),
                    })?;
                let mut rv: Option<HashSet<String>> = None;
                for case in cases {
                    let mut le = env.clone();
                    le.apply(&s);
                    let sm =
                        self.bind_pattern(&case.pattern, &apply_subst_type(&s, &tt), &mut le)?;
                    s = compose_subst(&s, &sm);
                    le.apply(&sm);
                    self.infer_body(&case.body, &mut le, er, ee)?;
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
                if let Some(vars) = rv {
                    env.linear_vars = vars;
                }
                Ok((s, Type::Unit))
            }
            Expr::Lambda {
                type_params,
                params,
                ret_type,
                effects,
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

                self.infer_body(body, &mut lambda_env, ret_type, effects)?;
                let remaining_lambda_linear: HashSet<String> = lambda_env
                    .linear_vars
                    .difference(&before_linear)
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
                            (
                                p.name.clone(),
                                self.convert_user_defined_to_var(&p.typ, &vars_set),
                            )
                        })
                        .collect(),
                    Box::new(self.convert_user_defined_to_var(ret_type, &vars_set)),
                    Box::new(self.convert_user_defined_to_var(effects, &vars_set)),
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
            Expr::External(_, typ) => Ok((HashMap::new(), typ.clone())),
            Expr::Raise(ex) => {
                let (s, t) = self.infer(env, ex, er, ee)?;
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
        match &p.node {
            Pattern::Variable(n, _) => {
                env.insert(
                    n.clone(),
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
                    Literal::String(_) => Type::String,
                    Literal::Unit => Type::Unit,
                };
                self.unify(tt, &tl).map_err(|m| TypeError {
                    message: m,
                    span: p.span.clone(),
                })
            }
            Pattern::Wildcard => {
                if env.contains_linear_type(tt) {
                    return Err(TypeError {
                        message: "Discard linear".into(),
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

    fn check_exhaustiveness(
        &self,
        env: &TypeEnv,
        tt: &Type,
        cases: &[MatchCase],
    ) -> Result<(), String> {
        let matrix: Vec<Vec<PatRef>> = cases
            .iter()
            .map(|c| vec![PatRef::Original(&c.pattern)])
            .collect();
        self.check_matrix(env, &matrix, &[tt])
    }

    fn check_matrix(
        &self,
        env: &TypeEnv,
        matrix: &[Vec<PatRef>],
        types: &[&Type],
    ) -> Result<(), String> {
        if matrix.is_empty() {
            return Err("Non-exhaustive".to_string());
        }
        if types.is_empty() {
            return Ok(());
        }
        if matrix.iter().any(|row| row.is_empty()) {
            return Err(
                "Internal typechecker error: malformed pattern matrix (empty row with remaining types)."
                    .to_string(),
            );
        }
        let ft = types[0];
        let rt = &types[1..];
        match ft {
            Type::Bool => {
                self.check_constructor_matrix(env, matrix, "true", 0, &[], rt)?;
                self.check_constructor_matrix(env, matrix, "false", 0, &[], rt)?;
                Ok(())
            }
            Type::Record(fields) => {
                let mut fts: Vec<&Type> = fields.iter().map(|(_, t)| t).collect();
                fts.extend_from_slice(rt);
                let mut nm = Vec::new();
                for row in matrix {
                    let Some((p, rr)) = row.split_first() else {
                        return Err(
                            "Internal typechecker error: record matrix row is empty.".into()
                        );
                    };
                    match p.node() {
                        Pattern::Record(pfs, open) => {
                            let mut nr = Vec::new();
                            for (fnm, _) in fields {
                                if let Some((_, pt)) = pfs.iter().find(|(n, _)| n == fnm) {
                                    nr.push(PatRef::Original(pt));
                                } else if *open {
                                    nr.push(PatRef::Synthetic(Pattern::Wildcard, p.span()));
                                } else {
                                    nr.push(PatRef::Synthetic(Pattern::Wildcard, p.span()));
                                }
                            }
                            nr.extend_from_slice(rr);
                            nm.push(nr);
                        }
                        Pattern::Wildcard | Pattern::Variable(..) => {
                            let mut nr =
                                vec![PatRef::Synthetic(Pattern::Wildcard, p.span()); fields.len()];
                            nr.extend_from_slice(rr);
                            nm.push(nr);
                        }
                        _ => {}
                    }
                }
                self.check_matrix(env, &nm, &fts)
            }
            Type::Unit => {
                self.check_constructor_matrix(env, matrix, "()", 0, &[], rt)?;
                Ok(())
            }
            Type::UserDefined(name, args) => {
                if matrix.iter().all(|row| {
                    row.first().map_or(false, |p| {
                        matches!(p.node(), Pattern::Wildcard | Pattern::Variable(..))
                    })
                }) {
                    return self.check_wildcard_matrix(env, matrix, rt);
                }
                if let Some(ed) = env.get_enum(name).cloned() {
                    let mut subst = HashMap::new();
                    for (p, a) in ed.type_params.iter().zip(args) {
                        subst.insert(p.clone(), a.clone());
                    }
                    for v in &ed.variants {
                        let ats: Vec<Type> = v
                            .fields
                            .iter()
                            .map(|(_, f)| apply_subst_type(&subst, f))
                            .collect();
                        let ars: Vec<&Type> = ats.iter().collect();
                        self.check_constructor_matrix(
                            env,
                            matrix,
                            &v.name,
                            v.fields.len(),
                            &ars,
                            rt,
                        )?;
                    }
                    Ok(())
                } else {
                    self.check_wildcard_matrix(env, matrix, rt)
                }
            }
            _ => self.check_wildcard_matrix(env, matrix, rt),
        }
    }

    fn check_wildcard_matrix(
        &self,
        env: &TypeEnv,
        matrix: &[Vec<PatRef>],
        rt: &[&Type],
    ) -> Result<(), String> {
        let mut nm = Vec::new();
        for row in matrix {
            let Some((head, rest)) = row.split_first() else {
                return Err("Internal typechecker error: wildcard matrix row is empty.".to_string());
            };
            match head.node() {
                Pattern::Wildcard | Pattern::Variable(..) => nm.push(rest.to_vec()),
                _ => {}
            }
        }
        if nm.is_empty() {
            return Err("Non-exhaustive".to_string());
        }
        self.check_matrix(env, &nm, rt)
    }

    fn check_constructor_matrix(
        &self,
        env: &TypeEnv,
        matrix: &[Vec<PatRef>],
        ctor: &str,
        arity: usize,
        ats: &[&Type],
        rt: &[&Type],
    ) -> Result<(), String> {
        let mut nm = Vec::new();
        let mut nt = ats.to_vec();
        nt.extend_from_slice(rt);
        for row in matrix {
            let Some((p, rest)) = row.split_first() else {
                return Err(
                    "Internal typechecker error: constructor matrix row is empty.".to_string(),
                );
            };
            match p.node() {
                Pattern::Constructor(c, args) => {
                    if c == ctor {
                        if args.len() != arity {
                            return Err(format!(
                                "Arity mismatch in pattern `{}`: expected {} fields, got {}.\nProvided pattern arguments: {}",
                                ctor,
                                arity,
                                args.len(),
                                summarize_ctor_args(args)
                            ));
                        }
                        let mut nr: Vec<PatRef> =
                            args.iter().map(|(_, a)| PatRef::Original(a)).collect();
                        nr.extend_from_slice(rest);
                        nm.push(nr);
                    }
                }
                Pattern::Literal(lit) => {
                    let name = match lit {
                        Literal::Bool(true) => "true",
                        Literal::Bool(false) => "false",
                        Literal::Unit => "()",
                        _ => "",
                    };
                    if name == ctor {
                        let mut nr = Vec::new();
                        nr.extend_from_slice(rest);
                        nm.push(nr);
                    }
                }
                Pattern::Wildcard | Pattern::Variable(..) => {
                    let mut nr = vec![PatRef::Synthetic(Pattern::Wildcard, p.span()); arity];
                    nr.extend_from_slice(rest);
                    nm.push(nr);
                }
                _ => {}
            }
        }
        self.check_matrix(env, &nm, &nt)
            .map_err(|_| format!("Non-exhaustive match: missing constructor `{}`.", ctor))
    }

    fn generalize_top_level(&self, func: &Function) -> Scheme {
        let vars: HashSet<String> = func.type_params.iter().cloned().collect();
        let pts: Vec<(String, Type)> = func
            .params
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    self.convert_user_defined_to_var(&p.typ, &vars),
                )
            })
            .collect();
        let rt = self.convert_user_defined_to_var(&func.ret_type, &vars);
        let ef = self.convert_user_defined_to_var(&func.effects, &vars);
        Scheme {
            vars: func.type_params.clone(),
            typ: Type::Arrow(pts, Box::new(rt), Box::new(ef)),
        }
    }

    fn generalize(&self, env: &TypeEnv, typ: Type) -> Scheme {
        let evs = get_free_vars_env(env);
        let tvs = get_free_vars_type(&typ);
        let free: Vec<String> = tvs.difference(&evs).cloned().collect();
        Scheme { vars: free, typ }
    }

    fn unify(&mut self, t1: &Type, t2: &Type) -> Result<Subst, String> {
        match (t1, t2) {
            (t1, t2) if t1 == t2 => Ok(HashMap::new()),
            (Type::IntLit, Type::I32)
            | (Type::I32, Type::IntLit)
            | (Type::IntLit, Type::I64)
            | (Type::I64, Type::IntLit) => Ok(HashMap::new()),
            (Type::FloatLit, Type::F32)
            | (Type::F32, Type::FloatLit)
            | (Type::FloatLit, Type::F64)
            | (Type::F64, Type::FloatLit) => Ok(HashMap::new()),
            (Type::Var(n), t) | (t, Type::Var(n)) => {
                if occurs_check(n, t) {
                    return Err("Recursive".into());
                }
                let mut s = HashMap::new();
                s.insert(n.clone(), t.clone());
                Ok(s)
            }
            (Type::Arrow(p1, r1, e1), Type::Arrow(p2, r2, e2)) => {
                if p1.len() != p2.len() {
                    return Err("Arity mismatch".into());
                }
                let mut s = HashMap::new();
                let mut remaining = p2.to_vec();
                for (n1, t1) in p1 {
                    let idx = remaining
                        .iter()
                        .position(|(n2, _)| n1 == n2)
                        .ok_or_else(|| format!("Label mismatch: missing {}", n1))?;
                    let (_, t2) = remaining.remove(idx);
                    let sn = self.unify(&apply_subst_type(&s, t1), &apply_subst_type(&s, &t2))?;
                    s = compose_subst(&s, &sn);
                }
                let sr = self.unify(&apply_subst_type(&s, r1), &apply_subst_type(&s, r2))?;
                s = compose_subst(&s, &sr);
                let se = self.unify(&apply_subst_type(&s, e1), &apply_subst_type(&s, e2))?;
                s = compose_subst(&s, &se);
                Ok(s)
            }
            (Type::Row(e1, t1), Type::Row(e2, t2)) => {
                let mut s = HashMap::new();
                let mut e2r = e2.clone();
                let mut e1r = Vec::new();
                for h1 in e1 {
                    let h1s = apply_subst_type(&s, h1);
                    if let Some(idx) = e2r.iter().position(|h2| h1s == apply_subst_type(&s, h2)) {
                        e2r.remove(idx);
                    } else {
                        e1r.push(h1.clone());
                    }
                }
                let ft = if t1.is_some() || t2.is_some() || !e1r.is_empty() || !e2r.is_empty() {
                    Some(Box::new(self.new_var()))
                } else {
                    None
                };
                if let Some(i) = t1 {
                    let row = Type::Row(e2r, ft.clone());
                    let sn = self.unify(&apply_subst_type(&s, i), &row)?;
                    s = compose_subst(&s, &sn);
                } else if !e2r.is_empty() {
                    return Err("Row mismatch".into());
                }
                if let Some(i) = t2 {
                    let row = Type::Row(e1r, ft);
                    let sn = self.unify(&apply_subst_type(&s, i), &row)?;
                    s = compose_subst(&s, &sn);
                } else if !e1r.is_empty() {
                    return Err("Row mismatch".into());
                }
                Ok(s)
            }
            (Type::Record(f1), Type::Record(f2)) => {
                if f1.len() != f2.len() {
                    return Err("Arity mismatch".into());
                }
                let (mut f1s, mut f2s) = (f1.clone(), f2.clone());
                f1s.sort_by(|a, b| a.0.cmp(&b.0));
                f2s.sort_by(|a, b| a.0.cmp(&b.0));
                let mut s = HashMap::new();
                for ((n1, t1), (n2, t2)) in f1s.iter().zip(f2s.iter()) {
                    if n1 != n2 {
                        return Err("Field mismatch".into());
                    }
                    let sn = self.unify(&apply_subst_type(&s, t1), &apply_subst_type(&s, t2))?;
                    s = compose_subst(&s, &sn);
                }
                Ok(s)
            }
            (Type::UserDefined(n1, a1), Type::UserDefined(n2, a2)) if n1 == n2 => {
                if a1.len() != a2.len() {
                    return Err("Arity mismatch".into());
                }
                let mut s = HashMap::new();
                for (a, b) in a1.iter().zip(a2) {
                    let sn = self.unify(&apply_subst_type(&s, a), &apply_subst_type(&s, b))?;
                    s = compose_subst(&s, &sn);
                }
                Ok(s)
            }
            (Type::Array(i1), Type::Array(i2)) => self.unify(i1, i2),
            (Type::Ref(t1), Type::Ref(t2))
            | (Type::Linear(t1), Type::Linear(t2))
            | (Type::Borrow(t1), Type::Borrow(t2)) => self.unify(t1, t2),
            // Borrow can be read as its underlying value type.
            (Type::Borrow(t1), t2) => self.unify(t1, t2),
            _ => Err(format!("Mismatch: {} vs {}", t1, t2)),
        }
    }
}

/// Applies a type substitution recursively to a type.
pub fn apply_subst_type(subst: &Subst, typ: &Type) -> Type {
    match typ {
        Type::Var(n) => subst.get(n).cloned().unwrap_or(typ.clone()),
        Type::Arrow(p, r, e) => Type::Arrow(
            p.iter()
                .map(|(n, t)| (n.clone(), apply_subst_type(subst, t)))
                .collect(),
            Box::new(apply_subst_type(subst, r)),
            Box::new(apply_subst_type(subst, e)),
        ),
        Type::UserDefined(n, a) => Type::UserDefined(
            n.clone(),
            a.iter().map(|t| apply_subst_type(subst, t)).collect(),
        ),
        Type::Ref(i) => Type::Ref(Box::new(apply_subst_type(subst, i))),
        Type::Linear(i) => Type::Linear(Box::new(apply_subst_type(subst, i))),
        Type::Borrow(i) => Type::Borrow(Box::new(apply_subst_type(subst, i))),
        Type::Array(i) => Type::Array(Box::new(apply_subst_type(subst, i))),
        Type::Row(es, t) => Type::Row(
            es.iter().map(|x| apply_subst_type(subst, x)).collect(),
            t.as_ref().map(|x| Box::new(apply_subst_type(subst, x))),
        ),
        Type::Record(fs) => Type::Record(
            fs.iter()
                .map(|(n, t)| (n.clone(), apply_subst_type(subst, t)))
                .collect(),
        ),
        _ => typ.clone(),
    }
}

fn compose_subst(s1: &Subst, s2: &Subst) -> Subst {
    let mut res = s2.clone();
    for (k, v) in s1 {
        res.insert(k.clone(), apply_subst_type(s2, v));
    }
    res
}

fn get_free_vars_type(typ: &Type) -> HashSet<String> {
    match typ {
        Type::Var(n) => {
            let mut s = HashSet::new();
            s.insert(n.clone());
            s
        }
        Type::Arrow(p, r, e) => {
            let mut s = get_free_vars_type(r);
            for (_, t) in p {
                s.extend(get_free_vars_type(t));
            }
            s.extend(get_free_vars_type(e));
            s
        }
        Type::UserDefined(_, a) => {
            let mut s = HashSet::new();
            for t in a {
                s.extend(get_free_vars_type(t));
            }
            s
        }
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::Array(i) => get_free_vars_type(i),
        Type::Row(es, t) => {
            let mut s = HashSet::new();
            for e in es {
                s.extend(get_free_vars_type(e));
            }
            if let Some(x) = t {
                s.extend(get_free_vars_type(x));
            }
            s
        }
        Type::Record(fs) => {
            let mut s = HashSet::new();
            for (_, t) in fs {
                s.extend(get_free_vars_type(t));
            }
            s
        }
        _ => HashSet::new(),
    }
}

fn get_free_vars_env(env: &TypeEnv) -> HashSet<String> {
    let mut s = HashSet::new();
    for sch in env.vars.values() {
        let ft = get_free_vars_type(&sch.typ);
        let b: HashSet<_> = sch.vars.iter().cloned().collect();
        s.extend(ft.difference(&b).cloned());
    }
    s
}

fn occurs_check(n: &str, t: &Type) -> bool {
    match t {
        Type::Var(m) => n == m,
        Type::Arrow(p, r, e) => {
            occurs_check(n, r) || p.iter().any(|(_, x)| occurs_check(n, x)) || occurs_check(n, e)
        }
        Type::UserDefined(_, a) => a.iter().any(|x| occurs_check(n, x)),
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::Array(i) => occurs_check(n, i),
        Type::Row(es, t) => {
            es.iter().any(|x| occurs_check(n, x))
                || t.as_ref().map_or(false, |x| occurs_check(n, x))
        }
        Type::Record(fs) => fs.iter().any(|(_, t)| occurs_check(n, t)),
        _ => false,
    }
}

fn contains_ref(t: &Type) -> bool {
    match t {
        Type::Ref(_) => true,
        Type::Arrow(p, r, e) => {
            contains_ref(r) || p.iter().any(|(_, x)| contains_ref(x)) || contains_ref(e)
        }
        Type::UserDefined(_, a) => a.iter().any(contains_ref),
        Type::Linear(i) | Type::Borrow(i) | Type::Array(i) => contains_ref(i),
        Type::Row(es, t) => {
            es.iter().any(contains_ref) || t.as_ref().map_or(false, |x| contains_ref(x))
        }
        Type::Record(fs) => fs.iter().any(|(_, t)| contains_ref(t)),
        _ => false,
    }
}

fn contains_exn_effect(t: &Type) -> bool {
    match t {
        Type::UserDefined(name, args) => {
            (name == "Exn" && args.is_empty()) || args.iter().any(contains_exn_effect)
        }
        Type::Arrow(params, ret, eff) => {
            params.iter().any(|(_, p)| contains_exn_effect(p))
                || contains_exn_effect(ret)
                || contains_exn_effect(eff)
        }
        Type::Ref(inner) | Type::Linear(inner) | Type::Borrow(inner) | Type::Array(inner) => {
            contains_exn_effect(inner)
        }
        Type::Row(effs, tail) => {
            effs.iter().any(contains_exn_effect)
                || tail.as_ref().is_some_and(|x| contains_exn_effect(x))
        }
        Type::Record(fields) => fields.iter().any(|(_, ft)| contains_exn_effect(ft)),
        _ => false,
    }
}

fn is_allowed_main_effect_signature(t: &Type) -> bool {
    match t {
        Type::Unit => true,
        Type::Row(effs, tail) => {
            if tail.is_some() {
                return false;
            }
            let mut has_io = false;
            let mut has_net = false;
            for eff in effs {
                match eff {
                    Type::UserDefined(name, args) if args.is_empty() => match name.as_str() {
                        "IO" => has_io = true,
                        "Net" => has_net = true,
                        _ => return false,
                    },
                    _ => return false,
                }
            }
            // Allow: {}, {IO}, {IO, Net}. Disallow: {Net}.
            !has_net || has_io
        }
        _ => false,
    }
}

fn lambda_references_name(body: &[Spanned<Stmt>], params: &[Param], name: &str) -> bool {
    let mut outer_keys = HashSet::new();
    outer_keys.insert(name.to_string());
    collect_lambda_captures(body, params, &outer_keys).contains(name)
}

fn collect_lambda_captures(
    body: &[Spanned<Stmt>],
    params: &[Param],
    outer_keys: &HashSet<String>,
) -> HashSet<String> {
    let mut bound_keys = HashSet::new();
    let mut bound_call_names = HashSet::new();
    for p in params {
        register_bound_name(&mut bound_keys, &mut bound_call_names, &p.name, &p.sigil);
    }
    let mut captures = HashSet::new();
    collect_stmt_captures(
        body,
        outer_keys,
        &bound_keys,
        &bound_call_names,
        &mut captures,
    );
    captures
}

fn register_bound_name(
    bound_keys: &mut HashSet<String>,
    bound_call_names: &mut HashSet<String>,
    name: &str,
    sigil: &Sigil,
) {
    bound_keys.insert(sigil.get_key(name));
    if matches!(sigil, Sigil::Immutable) {
        bound_call_names.insert(name.to_string());
    }
}

fn bind_pattern_names(
    pattern: &Spanned<Pattern>,
    bound_keys: &mut HashSet<String>,
    bound_call_names: &mut HashSet<String>,
) {
    match &pattern.node {
        Pattern::Variable(name, sigil) => {
            register_bound_name(bound_keys, bound_call_names, name, sigil);
        }
        Pattern::Constructor(_, args) => {
            for (_, arg) in args {
                bind_pattern_names(arg, bound_keys, bound_call_names);
            }
        }
        Pattern::Record(fields, _) => {
            for (_, pat) in fields {
                bind_pattern_names(pat, bound_keys, bound_call_names);
            }
        }
        Pattern::Literal(_) | Pattern::Wildcard => {}
    }
}

fn collect_stmt_captures(
    stmts: &[Spanned<Stmt>],
    outer_keys: &HashSet<String>,
    bound_keys: &HashSet<String>,
    bound_call_names: &HashSet<String>,
    captures: &mut HashSet<String>,
) {
    let mut local_bound_keys = bound_keys.clone();
    let mut local_bound_call_names = bound_call_names.clone();
    for stmt in stmts {
        match &stmt.node {
            Stmt::Let {
                name, sigil, value, ..
            } => {
                collect_expr_captures(
                    value,
                    outer_keys,
                    &local_bound_keys,
                    &local_bound_call_names,
                    captures,
                );
                register_bound_name(
                    &mut local_bound_keys,
                    &mut local_bound_call_names,
                    name,
                    sigil,
                );
            }
            Stmt::Expr(expr) | Stmt::Return(expr) | Stmt::Drop(expr) => {
                collect_expr_captures(
                    expr,
                    outer_keys,
                    &local_bound_keys,
                    &local_bound_call_names,
                    captures,
                );
            }
            Stmt::Assign { target, value } => {
                collect_expr_captures(
                    target,
                    outer_keys,
                    &local_bound_keys,
                    &local_bound_call_names,
                    captures,
                );
                collect_expr_captures(
                    value,
                    outer_keys,
                    &local_bound_keys,
                    &local_bound_call_names,
                    captures,
                );
            }
            Stmt::Conc(tasks) => {
                for task in tasks {
                    let mut task_bound_keys = HashSet::new();
                    let mut task_bound_call_names = HashSet::new();
                    for p in &task.params {
                        register_bound_name(
                            &mut task_bound_keys,
                            &mut task_bound_call_names,
                            &p.name,
                            &p.sigil,
                        );
                    }
                    collect_stmt_captures(
                        &task.body,
                        outer_keys,
                        &task_bound_keys,
                        &task_bound_call_names,
                        captures,
                    );
                }
            }
            Stmt::Try {
                body,
                catch_param,
                catch_body,
            } => {
                collect_stmt_captures(
                    body,
                    outer_keys,
                    &local_bound_keys,
                    &local_bound_call_names,
                    captures,
                );
                let mut catch_bound_keys = local_bound_keys.clone();
                let mut catch_bound_call_names = local_bound_call_names.clone();
                register_bound_name(
                    &mut catch_bound_keys,
                    &mut catch_bound_call_names,
                    catch_param,
                    &Sigil::Immutable,
                );
                collect_stmt_captures(
                    catch_body,
                    outer_keys,
                    &catch_bound_keys,
                    &catch_bound_call_names,
                    captures,
                );
            }
            Stmt::Comment => {}
        }
    }
}

fn collect_expr_captures(
    expr: &Spanned<Expr>,
    outer_keys: &HashSet<String>,
    bound_keys: &HashSet<String>,
    bound_call_names: &HashSet<String>,
    captures: &mut HashSet<String>,
) {
    match &expr.node {
        Expr::Literal(_) => {}
        Expr::Variable(name, sigil) | Expr::Borrow(name, sigil) => {
            let key = sigil.get_key(name);
            if outer_keys.contains(&key) && !bound_keys.contains(&key) {
                captures.insert(key);
            }
        }
        Expr::BinaryOp(lhs, _, rhs) | Expr::Index(lhs, rhs) => {
            collect_expr_captures(lhs, outer_keys, bound_keys, bound_call_names, captures);
            collect_expr_captures(rhs, outer_keys, bound_keys, bound_call_names, captures);
        }
        Expr::Call { func, args, .. } => {
            if !func.contains('.')
                && outer_keys.contains(func)
                && !bound_call_names.contains(func.as_str())
            {
                captures.insert(func.clone());
            }
            for (_, arg) in args {
                collect_expr_captures(arg, outer_keys, bound_keys, bound_call_names, captures);
            }
        }
        Expr::Constructor(_, args) => {
            for (_, arg) in args {
                collect_expr_captures(arg, outer_keys, bound_keys, bound_call_names, captures);
            }
        }
        Expr::Array(args) => {
            for arg in args {
                collect_expr_captures(arg, outer_keys, bound_keys, bound_call_names, captures);
            }
        }
        Expr::Record(fields) => {
            for (_, value) in fields {
                collect_expr_captures(value, outer_keys, bound_keys, bound_call_names, captures);
            }
        }
        Expr::FieldAccess(receiver, _) | Expr::Raise(receiver) => {
            collect_expr_captures(receiver, outer_keys, bound_keys, bound_call_names, captures);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_expr_captures(cond, outer_keys, bound_keys, bound_call_names, captures);
            collect_stmt_captures(
                then_branch,
                outer_keys,
                bound_keys,
                bound_call_names,
                captures,
            );
            if let Some(else_branch) = else_branch {
                collect_stmt_captures(
                    else_branch,
                    outer_keys,
                    bound_keys,
                    bound_call_names,
                    captures,
                );
            }
        }
        Expr::Match { target, cases } => {
            collect_expr_captures(target, outer_keys, bound_keys, bound_call_names, captures);
            for case in cases {
                let mut case_bound_keys = bound_keys.clone();
                let mut case_bound_call_names = bound_call_names.clone();
                bind_pattern_names(
                    &case.pattern,
                    &mut case_bound_keys,
                    &mut case_bound_call_names,
                );
                collect_stmt_captures(
                    &case.body,
                    outer_keys,
                    &case_bound_keys,
                    &case_bound_call_names,
                    captures,
                );
            }
        }
        Expr::Lambda { params, body, .. } => {
            let mut nested_bound_keys = bound_keys.clone();
            let mut nested_bound_call_names = bound_call_names.clone();
            for p in params {
                register_bound_name(
                    &mut nested_bound_keys,
                    &mut nested_bound_call_names,
                    &p.name,
                    &p.sigil,
                );
            }
            collect_stmt_captures(
                body,
                outer_keys,
                &nested_bound_keys,
                &nested_bound_call_names,
                captures,
            );
        }
        Expr::External(_, _) => {}
    }
}

#[derive(Clone)]
enum PatRef<'a> {
    Original(&'a Spanned<Pattern>),
    Synthetic(Pattern, Span),
}
impl<'a> PatRef<'a> {
    fn node(&self) -> &Pattern {
        match self {
            PatRef::Original(p) => &p.node,
            PatRef::Synthetic(p, _) => p,
        }
    }
    fn span(&self) -> Span {
        match self {
            PatRef::Original(p) => p.span.clone(),
            PatRef::Synthetic(_, s) => s.clone(),
        }
    }
}
