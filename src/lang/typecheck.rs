use super::ast::*;
use super::parser;
use crate::constants::{Permission, ENTRYPOINT};
use crate::lang::stdlib::load_stdlib_nx_programs;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

const EFFECT_EXN: &str = "Exn";

#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeWarning {
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

    pub fn check_unused_linear(&self, span: &Span) -> Result<(), TypeError> {
        if self.linear_vars.is_empty() {
            return Ok(());
        }
        let unused: Vec<_> = self.linear_vars
            .iter()
            .filter(|name| {
                if let Some(sch) = self.vars.get(*name) {
                    !is_auto_droppable(&sch.typ)
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        if !unused.is_empty() {
            return Err(TypeError {
                message: format!("Unused linear: {:?}", unused),
                span: span.clone(),
            });
        }
        Ok(())
    }

    fn contains_linear_type_inner(&self, typ: &Type, visiting: &mut HashSet<String>) -> bool {
        match typ {
            Type::Linear(_) | Type::Array(_) => true,
            Type::Borrow(_) => false,
            Type::Ref(inner) => self.contains_linear_type_inner(inner, visiting),
            Type::Arrow(_, _, _, _) => false,
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
    pub import_cache: HashMap<String, TypeEnv>,
    pub warnings: Vec<TypeWarning>,
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

pub fn list_enum_def() -> EnumDef {
    EnumDef {
        name: "List".to_string(),
        is_public: true,
        is_opaque: false,
        type_params: vec!["T".to_string()],
        variants: vec![
            VariantDef {
                name: "Nil".to_string(),
                fields: vec![],
            },
            VariantDef {
                name: "Cons".to_string(),
                fields: vec![
                    (Some("v".to_string()), Type::Var("T".to_string())),
                    (
                        Some("rest".to_string()),
                        Type::List(Box::new(Type::Var("T".to_string()))),
                    ),
                ],
            },
        ],
    }
}

pub fn exn_enum_def() -> EnumDef {
    EnumDef {
        name: "Exn".to_string(),
        is_public: true,
        is_opaque: false,
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
        Type::Arrow(p, r, req, e) => Type::Arrow(
            p.iter()
                .map(|(n, t)| (n.clone(), convert_generic_user_defined_to_var(t, vars)))
                .collect(),
            Box::new(convert_generic_user_defined_to_var(r, vars)),
            Box::new(convert_generic_user_defined_to_var(req, vars)),
            Box::new(convert_generic_user_defined_to_var(e, vars)),
        ),
        Type::Ref(i) => Type::Ref(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Linear(i) => Type::Linear(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Borrow(i) => Type::Borrow(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Array(i) => Type::Array(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::List(i) => Type::List(Box::new(convert_generic_user_defined_to_var(i, vars))),
        Type::Handler(name, req) => Type::Handler(
            name.clone(),
            Box::new(convert_generic_user_defined_to_var(req, vars)),
        ),
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
        is_opaque: ed.is_opaque,
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
        Type::Arrow(params, ret, req, eff) => Type::Arrow(
            params
                .iter()
                .map(|(name, t)| (name.clone(), default_numeric_literals(t)))
                .collect(),
            Box::new(default_numeric_literals(ret)),
            Box::new(default_numeric_literals(req)),
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
        Type::List(inner) => Type::List(Box::new(default_numeric_literals(inner))),
        Type::Handler(name, req) => Type::Handler(
            name.clone(),
            Box::new(default_numeric_literals(req)),
        ),
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

fn is_auto_droppable(typ: &Type) -> bool {
    match typ {
        Type::I32
        | Type::I64
        | Type::F32
        | Type::F64
        | Type::IntLit
        | Type::FloatLit
        | Type::Bool
        | Type::String
        | Type::Unit
        | Type::Array(_) => true,
        Type::Linear(inner) | Type::Borrow(inner) | Type::Ref(inner) => is_auto_droppable(inner),
        _ => false,
    }
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
        Type::Arrow(params, ret, _requires, _effects) => {
            // Skip requires/effects: their Row entries are port/effect names, not type variables.
            for (_, typ) in params {
                collect_external_type_vars(typ, env, out);
            }
            collect_external_type_vars(ret, env, out);
        }
        Type::Ref(inner) | Type::Linear(inner) | Type::Borrow(inner) | Type::Array(inner) | Type::List(inner) => {
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
        Type::Arrow(params, ret, requires, effects) => Type::Arrow(
            params
                .iter()
                .map(|(name, typ)| (name.clone(), convert_external_type_vars(typ, vars)))
                .collect(),
            Box::new(convert_external_type_vars(ret, vars)),
            Box::new(convert_external_type_vars(requires, vars)),
            Box::new(convert_external_type_vars(effects, vars)),
        ),
        Type::Ref(inner) => Type::Ref(Box::new(convert_external_type_vars(inner, vars))),
        Type::Linear(inner) => Type::Linear(Box::new(convert_external_type_vars(inner, vars))),
        Type::Borrow(inner) => Type::Borrow(Box::new(convert_external_type_vars(inner, vars))),
        Type::Array(inner) => Type::Array(Box::new(convert_external_type_vars(inner, vars))),
        Type::List(inner) => Type::List(Box::new(convert_external_type_vars(inner, vars))),
        Type::Handler(name, req) => Type::Handler(
            name.clone(),
            Box::new(convert_external_type_vars(req, vars)),
        ),
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

fn external_scheme(type_params: &[String], typ: &Type) -> Scheme {
    let vars_set: HashSet<String> = type_params.iter().cloned().collect();
    Scheme {
        typ: convert_external_type_vars(typ, &vars_set),
        vars: type_params.to_vec(),
    }
}

/// Check that all bare UserDefined names in `typ` that are not in `env.types`/`env.enums`
/// are declared in `type_params`. Returns an error listing unintroduced type variables.
fn check_unintroduced_type_vars(
    typ: &Type,
    type_params: &HashSet<String>,
    env: &TypeEnv,
) -> Result<(), String> {
    let mut found = HashSet::new();
    collect_external_type_vars(typ, env, &mut found);
    let unintroduced: Vec<String> = found
        .into_iter()
        .filter(|v| !type_params.contains(v))
        .collect();
    if unintroduced.is_empty() {
        Ok(())
    } else {
        let mut sorted = unintroduced;
        sorted.sort();
        Err(format!(
            "unintroduced type variable(s) in external binding: {}. Add them as explicit type parameters, e.g. <{}>",
            sorted.join(", "),
            sorted.join(", "),
        ))
    }
}

/// Register only types and enums from stdlib (not functions).
/// This makes core types like List<T> available without explicit import,
/// while functions (print, from_i64, etc.) require explicit import.
fn register_stdlib_types(env: &mut TypeEnv) {
    let Ok(programs) = load_stdlib_nx_programs() else {
        return;
    };

    for (_path, program) in programs {
        for def in &program.definitions {
            match &def.node {
                TopLevel::TypeDef(td) if td.is_public => {
                    let td_norm = normalize_typedef_generic_params(td);
                    env.types.insert(td_norm.name.clone(), td_norm);
                }
                TopLevel::Enum(ed) if ed.is_public => {
                    let ed_norm = normalize_enum_generic_params(ed);
                    if ed.is_opaque {
                        let opaque_ed = EnumDef {
                            name: ed_norm.name.clone(),
                            is_public: true,
                            is_opaque: true,
                            type_params: ed_norm.type_params.clone(),
                            variants: vec![],
                        };
                        env.enums.insert(opaque_ed.name.clone(), opaque_ed);
                    } else {
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
                }
                TopLevel::Exception(ex) if ex.is_public => {
                    let _ = register_exception_variant(env, ex, &def.span);
                }
                _ => {}
            }
        }
    }
}

impl TypeChecker {
    /// Creates a checker with only language-core builtins (no stdlib `.nx` imports).
    pub fn new_without_stdlib() -> Self {
        let mut env = TypeEnv::new();
        env.enums.insert(EFFECT_EXN.to_string(), exn_enum_def());
        env.enums.insert("List".to_string(), list_enum_def());
        register_nullary_variant_constructor(&mut env, "List", &["T".to_string()], &list_enum_def().variants[0]);

        env.linear_vars.clear();
        TypeChecker {
            supply: 0,
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

    fn new_var(&mut self) -> Type {
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

                        let src = fs::read_to_string(&import.path).map_err(|e| TypeError {
                            message: format!("Failed to read {}: {}", import.path, e),
                            span: def.span.clone(),
                        })?;
                        let p = parser::parser().parse(&src).map_err(|_| TypeError {
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
                                    // Ports are coeffects (environment requirements), not builtin effects.
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
                            requires,
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
                                            self.convert_user_defined_to_var(requires, &vars_set),
                                        ),
                                        Box::new(
                                            self.convert_user_defined_to_var(effects, &vars_set),
                                        ),
                                    ),
                                },
                            );
                        }
                        Expr::External(_, type_params, typ) => {
                            let vars_set: HashSet<String> =
                                type_params.iter().cloned().collect();
                            check_unintroduced_type_vars(typ, &vars_set, &self.env)
                                .map_err(|e| TypeError {
                                    message: e,
                                    span: gl.value.span.clone(),
                                })?;
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
                        let req = self.new_var();
                        let ef = self.new_var();
                        let sm = self
                            .unify(
                                &t,
                                &Type::Arrow(
                                    vec![],
                                    Box::new(Type::Unit),
                                    Box::new(req.clone()),
                                    Box::new(ef.clone()),
                                ),
                            )
                            .map_err(|_| TypeError {
                                message: "main must be a function '() -> unit'".into(),
                                span: def.span.clone(),
                            })?;
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
                        if contains_exn_effect(&final_ef) {
                            return Err(TypeError {
                                message: "main function must not declare Exn effect".into(),
                                span: def.span.clone(),
                            });
                        }
                        if !is_allowed_main_effect_signature(&final_ef) {
                            return Err(TypeError {
                                message: "main function effects must be {}".into(),
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
            let TopLevel::Let(gl) = &def.node else { continue };
            let Expr::Lambda {
                requires, body, ..
            } = &gl.value.node
            else {
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
            let mut missing: Vec<String> =
                used_reqs.difference(&declared_reqs).cloned().collect();
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

    fn collect_lint_warnings(&mut self, program: &Program) {
        self.collect_private_definition_warnings(program);
        self.collect_signature_minimization_warnings(program);
        for def in &program.definitions {
            if let TopLevel::Let(gl) = &def.node {
                match &gl.value.node {
                    Expr::Lambda { body, .. } => {
                        self.collect_unused_local_variable_warnings_in_function(&gl.name, body);
                    }
                    Expr::Handler {
                        coeffect_name,
                        functions,
                        ..
                    } => {
                        for f in functions {
                            let name = format!("handler {}.{}", coeffect_name, f.name);
                            self.collect_unused_local_variable_warnings_in_function(&name, &f.body);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn collect_private_definition_warnings(&mut self, program: &Program) {
        for def in &program.definitions {
            let TopLevel::Let(gl) = &def.node else {
                continue;
            };
            if gl.is_public || gl.name == ENTRYPOINT {
                continue;
            }
            let referenced_elsewhere = program.definitions.iter().any(|other| {
                let TopLevel::Let(other_gl) = &other.node else {
                    return false;
                };
                other_gl.name != gl.name && expr_mentions_name(&other_gl.value, &gl.name)
            });
            if !referenced_elsewhere {
                self.warnings.push(TypeWarning {
                    message: format!("Private definition '{}' is never referenced", gl.name),
                    span: def.span.clone(),
                });
            }
        }
    }

    fn collect_signature_minimization_warnings(&mut self, program: &Program) {
        for def in &program.definitions {
            let TopLevel::Let(gl) = &def.node else {
                continue;
            };
            let Expr::Lambda {
                requires,
                effects,
                body,
                ..
            } = &gl.value.node
            else {
                continue;
            };

            let (used_reqs, used_effs, unknown) =
                collect_signature_needs_from_stmts(body, &self.env);
            if unknown {
                continue;
            }

            let (declared_reqs, req_unknown) = extract_named_row_members(requires);
            if !req_unknown {
                let mut redundant_reqs: Vec<String> =
                    declared_reqs.difference(&used_reqs).cloned().collect();
                redundant_reqs.sort();
                if !redundant_reqs.is_empty() {
                    self.warnings.push(TypeWarning {
                        message: format!(
                            "Function '{}' declares reducible coeffects: {}",
                            gl.name,
                            redundant_reqs.join(", ")
                        ),
                        span: def.span.clone(),
                    });
                }
            }

            let (declared_effs, eff_unknown) = extract_named_row_members(effects);
            if !eff_unknown {
                let mut redundant_effs: Vec<String> =
                    declared_effs.difference(&used_effs).cloned().collect();
                redundant_effs.sort();
                if !redundant_effs.is_empty() {
                    self.warnings.push(TypeWarning {
                        message: format!(
                            "Function '{}' declares reducible effects: {}",
                            gl.name,
                            redundant_effs.join(", ")
                        ),
                        span: def.span.clone(),
                    });
                }
            }
        }
    }

    fn collect_unused_local_variable_warnings_in_function(
        &mut self,
        function_name: &str,
        body: &[Spanned<Stmt>],
    ) {
        let mut used = HashSet::new();
        collect_used_variable_keys_in_stmts(body, &mut used);
        let mut bindings = Vec::new();
        collect_local_let_bindings(body, &mut bindings);
        for (name, sigil, span) in bindings {
            if name.starts_with('_') || matches!(sigil, Sigil::Linear) {
                continue;
            }
            let key = sigil.get_key(&name);
            if !used.contains(&key) {
                self.warnings.push(TypeWarning {
                    message: format!(
                        "Unused local variable '{}' in function '{}'",
                        name, function_name
                    ),
                    span,
                });
            }
        }
        self.collect_unused_local_variable_warnings_in_stmts(body);
    }

    fn collect_unused_local_variable_warnings_in_stmts(&mut self, stmts: &[Spanned<Stmt>]) {
        for stmt in stmts {
            match &stmt.node {
                Stmt::Let { value, .. }
                | Stmt::Expr(value)
                | Stmt::Return(value) => {
                    self.collect_unused_local_variable_warnings_in_expr(value);
                }
                Stmt::Assign { target, value } => {
                    self.collect_unused_local_variable_warnings_in_expr(target);
                    self.collect_unused_local_variable_warnings_in_expr(value);
                }
                Stmt::Try {
                    body, catch_body, ..
                } => {
                    self.collect_unused_local_variable_warnings_in_stmts(body);
                    self.collect_unused_local_variable_warnings_in_stmts(catch_body);
                }
                Stmt::Inject { body, .. } => {
                    self.collect_unused_local_variable_warnings_in_stmts(body);
                }
                Stmt::Conc(tasks) => {
                    for task in tasks {
                        self.collect_unused_local_variable_warnings_in_function(
                            &format!("task {}", task.name),
                            &task.body,
                        );
                    }
                }
            }
        }
    }

    fn collect_unused_local_variable_warnings_in_expr(&mut self, expr: &Spanned<Expr>) {
        match &expr.node {
            Expr::BinaryOp(lhs, _, rhs) | Expr::Index(lhs, rhs) => {
                self.collect_unused_local_variable_warnings_in_expr(lhs);
                self.collect_unused_local_variable_warnings_in_expr(rhs);
            }
            Expr::Call { args, .. } => {
                for (_, arg) in args {
                    self.collect_unused_local_variable_warnings_in_expr(arg);
                }
            }
            Expr::Constructor(_, args) => {
                for (_, arg) in args {
                    self.collect_unused_local_variable_warnings_in_expr(arg);
                }
            }
            Expr::Record(fields) => {
                for (_, value) in fields {
                    self.collect_unused_local_variable_warnings_in_expr(value);
                }
            }
            Expr::Array(items) | Expr::List(items) => {
                for item in items {
                    self.collect_unused_local_variable_warnings_in_expr(item);
                }
            }
            Expr::FieldAccess(target, _) | Expr::Raise(target) => {
                self.collect_unused_local_variable_warnings_in_expr(target);
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                self.collect_unused_local_variable_warnings_in_expr(cond);
                self.collect_unused_local_variable_warnings_in_stmts(then_branch);
                if let Some(else_branch) = else_branch {
                    self.collect_unused_local_variable_warnings_in_stmts(else_branch);
                }
            }
            Expr::Match { target, cases } => {
                self.collect_unused_local_variable_warnings_in_expr(target);
                for case in cases {
                    self.collect_unused_local_variable_warnings_in_stmts(&case.body);
                }
            }
            Expr::Lambda { body, .. } => {
                self.collect_unused_local_variable_warnings_in_function("<lambda>", body);
            }
            Expr::Handler {
                coeffect_name,
                functions,
                ..
            } => {
                for f in functions {
                    let name = format!("handler {}.{}", coeffect_name, f.name);
                    self.collect_unused_local_variable_warnings_in_function(&name, &f.body);
                }
            }
            Expr::Literal(_) | Expr::Variable(_, _) | Expr::Borrow(_, _) | Expr::External(_, _, _) => {
            }
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
    /// declared requires — used for handler method bodies so that the
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
            &func.effects,
        )?;
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
            }
        }
        Ok(())
    }

    fn check_task(&mut self, t: &Function, oe: &TypeEnv, _span: &Span, outer_eq: &Type) -> Result<(), TypeError> {
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
        self.infer_body(&t.body, &mut te, &Type::Unit, &merged_requires, &t.effects)?;

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
                    BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
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
                    BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
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
                    BinaryOp::FEq | BinaryOp::FNe | BinaryOp::FLt | BinaryOp::FGt | BinaryOp::FLe | BinaryOp::FGe => {
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
                    BinaryOp::And | BinaryOp::Or => Err(TypeError {
                        message: format!("Operator {} is not available in source expressions", op),
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
                for case in cases {
                    let mut le = env.clone();
                    le.apply(&s);
                    let sm =
                        self.bind_pattern(&case.pattern, &apply_subst_type(&s, &tt), &mut le)?;
                    s = compose_subst(&s, &sm);
                    le.apply(&sm);
                    self.infer_body(&case.body, &mut le, er, eq, ee)?;
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
                requires,
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

                self.infer_body(body, &mut lambda_env, ret_type, requires, effects)?;
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
                        Box::new(f.effects.clone()),
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
                Ok((HashMap::new(), Type::Handler(coeffect_name.clone(), Box::new(handler_requires.clone()))))
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
        let ft_raw = types[0];
        let rt = &types[1..];
        // Unwrap Linear/Borrow wrappers to get the underlying structural type.
        let ft = match ft_raw {
            Type::Linear(inner) | Type::Borrow(inner) => inner.as_ref(),
            other => other,
        };
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
            Type::List(inner) => {
                // Delegate to UserDefined("List", [inner]) path
                let as_ud = Type::UserDefined("List".to_string(), vec![(**inner).clone()]);
                let mut types = vec![&as_ud];
                types.extend_from_slice(rt);
                self.check_matrix(env, matrix, &types)
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
            (Type::Arrow(p1, r1, req1, e1), Type::Arrow(p2, r2, req2, e2)) => {
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
                let sreq = self.unify(&apply_subst_type(&s, req1), &apply_subst_type(&s, req2))?;
                s = compose_subst(&s, &sreq);
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
            (Type::List(i1), Type::List(i2)) => self.unify(i1, i2),
            // Cross-unify List<T> with UserDefined("List", [T]) for stdlib compat
            (Type::List(inner), Type::UserDefined(n, args))
            | (Type::UserDefined(n, args), Type::List(inner))
                if n == "List" && args.len() == 1 =>
            {
                self.unify(inner, &args[0])
            }
            (Type::Ref(t1), Type::Ref(t2))
            | (Type::Linear(t1), Type::Linear(t2))
            | (Type::Borrow(t1), Type::Borrow(t2)) => self.unify(t1, t2),
            (Type::Handler(a, req_a), Type::Handler(b, req_b)) if a == b => self.unify(req_a, req_b),
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
        Type::Arrow(p, r, req, e) => Type::Arrow(
            p.iter()
                .map(|(n, t)| (n.clone(), apply_subst_type(subst, t)))
                .collect(),
            Box::new(apply_subst_type(subst, r)),
            Box::new(apply_subst_type(subst, req)),
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
        Type::List(i) => Type::List(Box::new(apply_subst_type(subst, i))),
        Type::Handler(name, req) => Type::Handler(
            name.clone(),
            Box::new(apply_subst_type(subst, req)),
        ),
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
        Type::Arrow(p, r, req, e) => {
            let mut s = get_free_vars_type(r);
            for (_, t) in p {
                s.extend(get_free_vars_type(t));
            }
            s.extend(get_free_vars_type(req));
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
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::Array(i) | Type::List(i) => get_free_vars_type(i),
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
        Type::Arrow(p, r, req, e) => {
            occurs_check(n, r)
                || p.iter().any(|(_, x)| occurs_check(n, x))
                || occurs_check(n, req)
                || occurs_check(n, e)
        }
        Type::UserDefined(_, a) => a.iter().any(|x| occurs_check(n, x)),
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::Array(i) | Type::List(i) => occurs_check(n, i),
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
        Type::Arrow(p, r, req, e) => {
            contains_ref(r)
                || p.iter().any(|(_, x)| contains_ref(x))
                || contains_ref(req)
                || contains_ref(e)
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

fn strip_required_port_coeffect(t: &Type, coeffect_name: &str) -> Type {
    match t {
        Type::Arrow(params, ret, req, eff) => Type::Arrow(
            params.clone(),
            ret.clone(),
            Box::new(strip_required_port_coeffect(req, coeffect_name)),
            eff.clone(),
        ),
        Type::Row(reqs, tail) => Type::Row(
            reqs.iter()
                .filter(|req| {
                    !matches!(
                        req,
                        Type::UserDefined(name, args)
                            if args.is_empty() && name == coeffect_name
                    )
                })
                .cloned()
                .collect(),
            tail.clone(),
        ),
        other => other.clone(),
    }
}

fn contains_exn_effect(t: &Type) -> bool {
    contains_named_effect(t, EFFECT_EXN)
}

/// Merges two requirement row types, deduplicating entries.
fn merge_type_rows(base: &Type, extra: &Type) -> Type {
    let base_items = match base {
        Type::Row(items, _) => items.clone(),
        Type::Unit => vec![],
        _ => return base.clone(),
    };
    let extra_items = match extra {
        Type::Row(items, _) => items.clone(),
        Type::Unit => vec![],
        _ => return base.clone(),
    };
    let mut merged = base_items;
    for item in extra_items {
        if !merged.contains(&item) {
            merged.push(item);
        }
    }
    let tail = match base {
        Type::Row(_, t) => t.clone(),
        _ => None,
    };
    if merged.is_empty() && tail.is_none() {
        Type::Unit
    } else {
        Type::Row(merged, tail)
    }
}

/// Extracts port names from a handler's require row type.
fn extract_row_port_names(t: &Type) -> Vec<String> {
    match t {
        Type::Row(reqs, _) => reqs
            .iter()
            .filter_map(|r| match r {
                Type::UserDefined(name, args) if args.is_empty() => Some(name.clone()),
                _ => None,
            })
            .collect(),
        Type::Unit => vec![],
        _ => vec![],
    }
}

fn contains_named_effect(t: &Type, effect_name: &str) -> bool {
    match t {
        Type::UserDefined(name, args) => {
            (name == effect_name && args.is_empty())
                || args
                    .iter()
                    .any(|inner| contains_named_effect(inner, effect_name))
        }
        Type::Arrow(params, ret, req, eff) => {
            params
                .iter()
                .any(|(_, p)| contains_named_effect(p, effect_name))
                || contains_named_effect(ret, effect_name)
                || contains_named_effect(req, effect_name)
                || contains_named_effect(eff, effect_name)
        }
        Type::Ref(inner) | Type::Linear(inner) | Type::Borrow(inner) | Type::Array(inner) => {
            contains_named_effect(inner, effect_name)
        }
        Type::Row(effs, tail) => {
            effs.iter().any(|e| contains_named_effect(e, effect_name))
                || tail
                    .as_ref()
                    .is_some_and(|x| contains_named_effect(x, effect_name))
        }
        Type::Record(fields) => fields
            .iter()
            .any(|(_, ft)| contains_named_effect(ft, effect_name)),
        _ => false,
    }
}

fn is_allowed_main_effect_signature(t: &Type) -> bool {
    match t {
        Type::Unit => true,
        Type::Row(effs, tail) => tail.is_none() && effs.is_empty(),
        _ => false,
    }
}

fn is_allowed_main_require_signature(t: &Type) -> bool {
    match t {
        Type::Unit => true,
        Type::Row(reqs, tail) => {
            tail.is_none()
                && reqs.iter().all(|r| matches!(r,
                    Type::UserDefined(name, args) if args.is_empty()
                        && is_known_runtime_perm(name)
                ))
        }
        _ => false,
    }
}

fn is_known_runtime_perm(name: &str) -> bool {
    Permission::from_perm_name(name).is_some()
}

fn find_private_type_in_public_signature(typ: &Type, env: &TypeEnv) -> Option<String> {
    match typ {
        Type::UserDefined(name, args) => {
            if env.types.get(name).is_some_and(|td| !td.is_public) {
                return Some(name.clone());
            }
            if env.enums.get(name).is_some_and(|ed| !ed.is_public) {
                return Some(name.clone());
            }
            for arg in args {
                if let Some(found) = find_private_type_in_public_signature(arg, env) {
                    return Some(found);
                }
            }
            None
        }
        Type::Arrow(params, ret, req, eff) => {
            for (_, param_type) in params {
                if let Some(found) = find_private_type_in_public_signature(param_type, env) {
                    return Some(found);
                }
            }
            find_private_type_in_public_signature(ret, env)
                .or_else(|| find_private_type_in_public_signature(req, env))
                .or_else(|| find_private_type_in_public_signature(eff, env))
        }
        Type::Ref(inner) | Type::Linear(inner) | Type::Borrow(inner) | Type::Array(inner) => {
            find_private_type_in_public_signature(inner, env)
        }
        Type::Row(effs, tail) => {
            for eff in effs {
                if let Some(found) = find_private_type_in_public_signature(eff, env) {
                    return Some(found);
                }
            }
            tail.as_ref()
                .and_then(|row_tail| find_private_type_in_public_signature(row_tail, env))
        }
        Type::Record(fields) => {
            for (_, field_type) in fields {
                if let Some(found) = find_private_type_in_public_signature(field_type, env) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn extract_named_row_members(row: &Type) -> (HashSet<String>, bool) {
    match row {
        Type::Unit => (HashSet::new(), false),
        Type::Row(items, tail) => {
            let mut names = HashSet::new();
            let mut unknown = tail.is_some();
            for item in items {
                match item {
                    Type::UserDefined(name, args) if args.is_empty() => {
                        names.insert(name.clone());
                    }
                    _ => {
                        unknown = true;
                    }
                }
            }
            (names, unknown)
        }
        _ => (HashSet::new(), true),
    }
}

fn lookup_call_signature(
    func: &str,
    env: &TypeEnv,
) -> Option<(HashSet<String>, HashSet<String>, bool)> {
    let scheme = env.vars.get(func).or_else(|| {
        let (module_name, item_name) = func.split_once('.')?;
        env.modules.get(module_name)?.vars.get(item_name)
    })?;

    let arrow = match &scheme.typ {
        Type::Arrow(_, _, req, eff) => Some((req.as_ref(), eff.as_ref())),
        Type::Linear(inner) => match inner.as_ref() {
            Type::Arrow(_, _, req, eff) => Some((req.as_ref(), eff.as_ref())),
            _ => None,
        },
        _ => None,
    }?;

    let (reqs, req_unknown) = extract_named_row_members(arrow.0);
    let (effs, eff_unknown) = extract_named_row_members(arrow.1);
    Some((reqs, effs, req_unknown || eff_unknown))
}

fn collect_signature_needs_from_stmts(
    stmts: &[Spanned<Stmt>],
    env: &TypeEnv,
) -> (HashSet<String>, HashSet<String>, bool) {
    let mut reqs = HashSet::new();
    let mut effs = HashSet::new();
    let mut unknown = false;

    for stmt in stmts {
        match &stmt.node {
            Stmt::Let { value, .. }
            | Stmt::Expr(value)
            | Stmt::Return(value) => {
                let (inner_reqs, inner_effs, inner_unknown) =
                    collect_signature_needs_from_expr(value, env);
                reqs.extend(inner_reqs);
                effs.extend(inner_effs);
                unknown |= inner_unknown;
            }
            Stmt::Assign { target, value } => {
                let (lhs_reqs, lhs_effs, lhs_unknown) =
                    collect_signature_needs_from_expr(target, env);
                reqs.extend(lhs_reqs);
                effs.extend(lhs_effs);
                unknown |= lhs_unknown;

                let (rhs_reqs, rhs_effs, rhs_unknown) =
                    collect_signature_needs_from_expr(value, env);
                reqs.extend(rhs_reqs);
                effs.extend(rhs_effs);
                unknown |= rhs_unknown;
            }
            Stmt::Conc(tasks) => {
                for task in tasks {
                    let (task_reqs, task_effs, task_unknown) =
                        collect_signature_needs_from_stmts(&task.body, env);
                    reqs.extend(task_reqs);
                    effs.extend(task_effs);
                    unknown |= task_unknown;
                }
            }
            Stmt::Try {
                body, catch_body, ..
            } => {
                let (body_reqs, mut body_effs, body_unknown) =
                    collect_signature_needs_from_stmts(body, env);
                let (catch_reqs, catch_effs, catch_unknown) =
                    collect_signature_needs_from_stmts(catch_body, env);
                body_effs.remove(EFFECT_EXN);
                reqs.extend(body_reqs);
                reqs.extend(catch_reqs);
                effs.extend(body_effs);
                effs.extend(catch_effs);
                unknown |= body_unknown || catch_unknown;
            }
            Stmt::Inject { handlers, body } => {
                let (mut body_reqs, body_effs, body_unknown) =
                    collect_signature_needs_from_stmts(body, env);
                let mut injected = HashSet::new();
                let mut handler_extra_reqs = HashSet::new();
                for handler_name in handlers {
                    if let Some(scheme) = env.get(handler_name) {
                        match &scheme.typ {
                            Type::Handler(name, req) => {
                                injected.insert(name.clone());
                                for r in extract_row_port_names(req) {
                                    handler_extra_reqs.insert(r);
                                }
                            }
                            _ => unknown = true,
                        }
                    } else {
                        unknown = true;
                    }
                }
                body_reqs.retain(|name| !injected.contains(name));
                body_reqs.extend(handler_extra_reqs);
                reqs.extend(body_reqs);
                effs.extend(body_effs);
                unknown |= body_unknown;
            }
        }
    }

    (reqs, effs, unknown)
}

fn collect_signature_needs_from_expr(
    expr: &Spanned<Expr>,
    env: &TypeEnv,
) -> (HashSet<String>, HashSet<String>, bool) {
    match &expr.node {
        Expr::Call { func, args } => {
            let mut reqs = HashSet::new();
            let mut effs = HashSet::new();
            let mut unknown = false;
            for (_, arg) in args {
                let (inner_reqs, inner_effs, inner_unknown) =
                    collect_signature_needs_from_expr(arg, env);
                reqs.extend(inner_reqs);
                effs.extend(inner_effs);
                unknown |= inner_unknown;
            }
            if let Some((call_reqs, call_effs, call_unknown)) = lookup_call_signature(func, env) {
                reqs.extend(call_reqs);
                effs.extend(call_effs);
                unknown |= call_unknown;
            } else {
                unknown = true;
            }
            (reqs, effs, unknown)
        }
        Expr::Raise(inner) => {
            let (reqs, mut effs, unknown) = collect_signature_needs_from_expr(inner, env);
            effs.insert(EFFECT_EXN.to_string());
            (reqs, effs, unknown)
        }
        Expr::BinaryOp(lhs, _, rhs) | Expr::Index(lhs, rhs) => {
            let (mut reqs, mut effs, mut unknown) = collect_signature_needs_from_expr(lhs, env);
            let (rhs_reqs, rhs_effs, rhs_unknown) = collect_signature_needs_from_expr(rhs, env);
            reqs.extend(rhs_reqs);
            effs.extend(rhs_effs);
            unknown |= rhs_unknown;
            (reqs, effs, unknown)
        }
        Expr::Constructor(_, args) => {
            let mut reqs = HashSet::new();
            let mut effs = HashSet::new();
            let mut unknown = false;
            for (_, arg) in args {
                let (inner_reqs, inner_effs, inner_unknown) =
                    collect_signature_needs_from_expr(arg, env);
                reqs.extend(inner_reqs);
                effs.extend(inner_effs);
                unknown |= inner_unknown;
            }
            (reqs, effs, unknown)
        }
        Expr::Record(fields) => {
            let mut reqs = HashSet::new();
            let mut effs = HashSet::new();
            let mut unknown = false;
            for (_, value) in fields {
                let (inner_reqs, inner_effs, inner_unknown) =
                    collect_signature_needs_from_expr(value, env);
                reqs.extend(inner_reqs);
                effs.extend(inner_effs);
                unknown |= inner_unknown;
            }
            (reqs, effs, unknown)
        }
        Expr::Array(items) | Expr::List(items) => {
            let mut reqs = HashSet::new();
            let mut effs = HashSet::new();
            let mut unknown = false;
            for item in items {
                let (inner_reqs, inner_effs, inner_unknown) =
                    collect_signature_needs_from_expr(item, env);
                reqs.extend(inner_reqs);
                effs.extend(inner_effs);
                unknown |= inner_unknown;
            }
            (reqs, effs, unknown)
        }
        Expr::FieldAccess(target, _) => collect_signature_needs_from_expr(target, env),
        Expr::Borrow(_, _) => (HashSet::new(), HashSet::new(), false),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            let (mut reqs, mut effs, mut unknown) = collect_signature_needs_from_expr(cond, env);
            let (then_reqs, then_effs, then_unknown) =
                collect_signature_needs_from_stmts(then_branch, env);
            reqs.extend(then_reqs);
            effs.extend(then_effs);
            unknown |= then_unknown;
            if let Some(else_branch) = else_branch {
                let (else_reqs, else_effs, else_unknown) =
                    collect_signature_needs_from_stmts(else_branch, env);
                reqs.extend(else_reqs);
                effs.extend(else_effs);
                unknown |= else_unknown;
            }
            (reqs, effs, unknown)
        }
        Expr::Match { target, cases } => {
            let (mut reqs, mut effs, mut unknown) = collect_signature_needs_from_expr(target, env);
            for case in cases {
                let (case_reqs, case_effs, case_unknown) =
                    collect_signature_needs_from_stmts(&case.body, env);
                reqs.extend(case_reqs);
                effs.extend(case_effs);
                unknown |= case_unknown;
            }
            (reqs, effs, unknown)
        }
        // Nested closures/handlers are definitions; they don't imply this function's immediate signature needs.
        Expr::Lambda { .. } | Expr::Handler { .. } => (HashSet::new(), HashSet::new(), false),
        Expr::Literal(_) | Expr::Variable(_, _) | Expr::External(_, _, _) => {
            (HashSet::new(), HashSet::new(), false)
        }
    }
}

fn expr_mentions_name(expr: &Spanned<Expr>, target: &str) -> bool {
    match &expr.node {
        Expr::Variable(name, sigil) => matches!(sigil, Sigil::Immutable) && name == target,
        Expr::Call { func, args } => {
            (func == target || (func.split_once('.').is_none() && func == target))
                || args.iter().any(|(_, arg)| expr_mentions_name(arg, target))
        }
        Expr::Borrow(name, sigil) => matches!(sigil, Sigil::Immutable) && name == target,
        Expr::BinaryOp(lhs, _, rhs) | Expr::Index(lhs, rhs) => {
            expr_mentions_name(lhs, target) || expr_mentions_name(rhs, target)
        }
        Expr::Constructor(_, args) => args.iter().any(|(_, arg)| expr_mentions_name(arg, target)),
        Expr::Record(fields) => fields
            .iter()
            .any(|(_, arg)| expr_mentions_name(arg, target)),
        Expr::Array(items) | Expr::List(items) => items.iter().any(|item| expr_mentions_name(item, target)),
        Expr::FieldAccess(receiver, _) | Expr::Raise(receiver) => {
            expr_mentions_name(receiver, target)
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            expr_mentions_name(cond, target)
                || then_branch
                    .iter()
                    .any(|stmt| stmt_mentions_name(stmt, target))
                || else_branch.as_ref().is_some_and(|branch| {
                    branch.iter().any(|stmt| stmt_mentions_name(stmt, target))
                })
        }
        Expr::Match {
            target: mtarget,
            cases,
        } => {
            expr_mentions_name(mtarget, target)
                || cases.iter().any(|case| {
                    case.body
                        .iter()
                        .any(|stmt| stmt_mentions_name(stmt, target))
                })
        }
        Expr::Lambda { body, .. } => body.iter().any(|stmt| stmt_mentions_name(stmt, target)),
        Expr::Handler { functions, .. } => functions
            .iter()
            .any(|f| f.body.iter().any(|stmt| stmt_mentions_name(stmt, target))),
        Expr::Literal(_) | Expr::External(_, _, _) => false,
    }
}

fn stmt_mentions_name(stmt: &Spanned<Stmt>, target: &str) -> bool {
    match &stmt.node {
        Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::Return(value) => {
            expr_mentions_name(value, target)
        }
        Stmt::Assign { target: lhs, value } => {
            expr_mentions_name(lhs, target) || expr_mentions_name(value, target)
        }
        Stmt::Conc(tasks) => tasks.iter().any(|task| {
            task.body
                .iter()
                .any(|stmt| stmt_mentions_name(stmt, target))
        }),
        Stmt::Try {
            body, catch_body, ..
        } => {
            body.iter().any(|stmt| stmt_mentions_name(stmt, target))
                || catch_body
                    .iter()
                    .any(|stmt| stmt_mentions_name(stmt, target))
        }
        Stmt::Inject { handlers, body } => {
            handlers.iter().any(|h| {
                h == target || h.starts_with(&format!("{}.", target))
            }) || body.iter().any(|stmt| stmt_mentions_name(stmt, target))
        }
    }
}

fn collect_used_variable_keys_in_stmts(stmts: &[Spanned<Stmt>], out: &mut HashSet<String>) {
    for stmt in stmts {
        match &stmt.node {
            Stmt::Let { value, .. }
            | Stmt::Expr(value)
            | Stmt::Return(value) => {
                collect_used_variable_keys_in_expr(value, out);
            }
            Stmt::Assign { target, value } => {
                collect_used_variable_keys_in_expr(target, out);
                collect_used_variable_keys_in_expr(value, out);
            }
            Stmt::Conc(tasks) => {
                for task in tasks {
                    collect_used_variable_keys_in_stmts(&task.body, out);
                }
            }
            Stmt::Try {
                body, catch_body, ..
            } => {
                collect_used_variable_keys_in_stmts(body, out);
                collect_used_variable_keys_in_stmts(catch_body, out);
            }
            Stmt::Inject { handlers, body } => {
                for handler in handlers {
                    if let Some((mod_part, _)) = handler.split_once('.') {
                        out.insert(mod_part.to_string());
                    } else {
                        out.insert(handler.clone());
                    }
                }
                collect_used_variable_keys_in_stmts(body, out);
            }
        }
    }
}

fn collect_used_variable_keys_in_expr(expr: &Spanned<Expr>, out: &mut HashSet<String>) {
    match &expr.node {
        Expr::Variable(name, sigil) | Expr::Borrow(name, sigil) => {
            out.insert(sigil.get_key(name));
        }
        Expr::Call { func, args } => {
            if !func.contains('.') {
                out.insert(func.clone());
            }
            for (_, arg) in args {
                collect_used_variable_keys_in_expr(arg, out);
            }
        }
        Expr::BinaryOp(lhs, _, rhs) | Expr::Index(lhs, rhs) => {
            collect_used_variable_keys_in_expr(lhs, out);
            collect_used_variable_keys_in_expr(rhs, out);
        }
        Expr::Constructor(_, args) => {
            for (_, arg) in args {
                collect_used_variable_keys_in_expr(arg, out);
            }
        }
        Expr::Record(fields) => {
            for (_, value) in fields {
                collect_used_variable_keys_in_expr(value, out);
            }
        }
        Expr::Array(items) | Expr::List(items) => {
            for item in items {
                collect_used_variable_keys_in_expr(item, out);
            }
        }
        Expr::FieldAccess(target, _) | Expr::Raise(target) => {
            collect_used_variable_keys_in_expr(target, out);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_used_variable_keys_in_expr(cond, out);
            collect_used_variable_keys_in_stmts(then_branch, out);
            if let Some(else_branch) = else_branch {
                collect_used_variable_keys_in_stmts(else_branch, out);
            }
        }
        Expr::Match { target, cases } => {
            collect_used_variable_keys_in_expr(target, out);
            for case in cases {
                collect_used_variable_keys_in_stmts(&case.body, out);
            }
        }
        Expr::Lambda { body, .. } => collect_used_variable_keys_in_stmts(body, out),
        Expr::Handler { functions, .. } => {
            for f in functions {
                collect_used_variable_keys_in_stmts(&f.body, out);
            }
        }
        Expr::Literal(_) | Expr::External(_, _, _) => {}
    }
}

fn collect_local_let_bindings(stmts: &[Spanned<Stmt>], out: &mut Vec<(String, Sigil, Span)>) {
    for stmt in stmts {
        match &stmt.node {
            Stmt::Let {
                name, sigil, value, ..
            } => {
                out.push((name.clone(), sigil.clone(), stmt.span.clone()));
                collect_local_let_bindings_in_expr(value, out);
            }
            Stmt::Expr(value) | Stmt::Return(value) => {
                collect_local_let_bindings_in_expr(value, out);
            }
            Stmt::Assign { target, value } => {
                collect_local_let_bindings_in_expr(target, out);
                collect_local_let_bindings_in_expr(value, out);
            }
            Stmt::Try {
                body, catch_body, ..
            } => {
                collect_local_let_bindings(body, out);
                collect_local_let_bindings(catch_body, out);
            }
            Stmt::Inject { body, .. } => collect_local_let_bindings(body, out),
            Stmt::Conc(tasks) => {
                for task in tasks {
                    collect_local_let_bindings(&task.body, out);
                }
            }
        }
    }
}

fn collect_local_let_bindings_in_expr(expr: &Spanned<Expr>, out: &mut Vec<(String, Sigil, Span)>) {
    match &expr.node {
        Expr::BinaryOp(lhs, _, rhs) | Expr::Index(lhs, rhs) => {
            collect_local_let_bindings_in_expr(lhs, out);
            collect_local_let_bindings_in_expr(rhs, out);
        }
        Expr::Call { args, .. } => {
            for (_, arg) in args {
                collect_local_let_bindings_in_expr(arg, out);
            }
        }
        Expr::Constructor(_, args) => {
            for (_, arg) in args {
                collect_local_let_bindings_in_expr(arg, out);
            }
        }
        Expr::Record(fields) => {
            for (_, value) in fields {
                collect_local_let_bindings_in_expr(value, out);
            }
        }
        Expr::Array(items) | Expr::List(items) => {
            for item in items {
                collect_local_let_bindings_in_expr(item, out);
            }
        }
        Expr::FieldAccess(target, _) | Expr::Raise(target) => {
            collect_local_let_bindings_in_expr(target, out);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_local_let_bindings_in_expr(cond, out);
            collect_local_let_bindings(then_branch, out);
            if let Some(else_branch) = else_branch {
                collect_local_let_bindings(else_branch, out);
            }
        }
        Expr::Match { target, cases } => {
            collect_local_let_bindings_in_expr(target, out);
            for case in cases {
                collect_local_let_bindings(&case.body, out);
            }
        }
        // Nested functions/handlers are analyzed separately.
        Expr::Lambda { .. } | Expr::Handler { .. } => {}
        Expr::Literal(_) | Expr::Variable(_, _) | Expr::Borrow(_, _) | Expr::External(_, _, _) => {}
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
            Stmt::Expr(expr) | Stmt::Return(expr) => {
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
            Stmt::Inject { handlers: _, body } => {
                collect_stmt_captures(
                    body,
                    outer_keys,
                    &local_bound_keys,
                    &local_bound_call_names,
                    captures,
                );
            }
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
        Expr::Array(args) | Expr::List(args) => {
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
        Expr::External(_, _, _) => {}
        Expr::Handler { functions, .. } => {
            for f in functions {
                let mut fn_bound_keys = HashSet::new();
                let mut fn_bound_call_names = HashSet::new();
                for p in &f.params {
                    register_bound_name(
                        &mut fn_bound_keys,
                        &mut fn_bound_call_names,
                        &p.name,
                        &p.sigil,
                    );
                }
                collect_stmt_captures(
                    &f.body,
                    outer_keys,
                    &fn_bound_keys,
                    &fn_bound_call_names,
                    captures,
                );
            }
        }
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
