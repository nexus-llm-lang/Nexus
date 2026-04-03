use super::env::{Scheme, TypeEnv, TypeError};
use crate::constants::Permission;
use crate::lang::ast::*;
use crate::lang::stdlib::load_stdlib_nx_programs;
use std::collections::HashSet;
use std::path::Path;

pub(super) fn get_default_alias(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

pub(super) fn describe_ctor_field(label: &Option<String>, index: usize) -> String {
    match label {
        Some(name) => format!("#{} label `{}`", index + 1, name),
        None => format!("#{} positional field", index + 1),
    }
}

pub(super) fn summarize_ctor_args<T>(args: &[(Option<String>, T)]) -> String {
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

pub(super) fn summarize_ctor_fields(fields: &[(Option<String>, Type)]) -> String {
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

pub(super) fn register_nullary_variant_constructor(
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

pub(super) fn register_exception_variant(
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
            // Already registered (e.g. auto-loaded from stdlib) — skip silently
            return Ok(());
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

/// Import a variant by name from a public_env into the target env.
/// Searches all enums (including Exn) in public_env for a variant matching `name`.
/// If found, imports the entire parent enum into target_env (merging if already present).
/// Returns true if the name matched a variant.
pub(super) fn import_variant_by_name(
    target_env: &mut TypeEnv,
    public_env: &TypeEnv,
    name: &str,
) -> bool {
    // Find the enum containing this variant
    let found = public_env.enums.iter().find_map(|(enum_name, ed)| {
        if ed.variants.iter().any(|v| v.name == name) {
            Some((enum_name.clone(), ed.clone()))
        } else {
            None
        }
    });
    let Some((enum_name, ed)) = found else {
        return false;
    };
    // Collect new variants to add
    let existing: Vec<String> = target_env
        .enums
        .get(&enum_name)
        .map(|e| e.variants.iter().map(|v| v.name.clone()).collect())
        .unwrap_or_default();
    let new_variants: Vec<_> = ed
        .variants
        .iter()
        .filter(|v| !existing.contains(&v.name))
        .cloned()
        .collect();
    // Register nullary constructors first (before borrowing enums mutably)
    for v in &new_variants {
        register_nullary_variant_constructor(target_env, &enum_name, &ed.type_params, v);
    }
    // Now insert/merge enum
    let target_ed = target_env
        .enums
        .entry(enum_name.clone())
        .or_insert_with(|| EnumDef {
            name: enum_name,
            is_public: ed.is_public,
            is_opaque: ed.is_opaque,
            type_params: ed.type_params.clone(),
            variants: vec![],
        });
    for v in new_variants {
        target_ed.variants.push(v);
    }
    true
}

pub(super) fn convert_generic_user_defined_to_var(typ: &Type, vars: &HashSet<String>) -> Type {
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
        Type::Lazy(i) => Type::Lazy(Box::new(convert_generic_user_defined_to_var(i, vars))),
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

pub(super) fn normalize_typedef_generic_params(td: &TypeDef) -> TypeDef {
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

pub(super) fn normalize_enum_generic_params(ed: &EnumDef) -> EnumDef {
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

pub(super) fn default_numeric_literals(typ: &Type) -> Type {
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
        Type::Lazy(inner) => Type::Lazy(Box::new(default_numeric_literals(inner))),
        Type::Borrow(inner) => Type::Borrow(Box::new(default_numeric_literals(inner))),
        Type::Array(inner) => Type::Array(Box::new(default_numeric_literals(inner))),
        Type::List(inner) => Type::List(Box::new(default_numeric_literals(inner))),
        Type::Handler(name, req) => {
            Type::Handler(name.clone(), Box::new(default_numeric_literals(req)))
        }
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

pub(super) fn select_int_type(left: &Type, right: &Type) -> Option<Type> {
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

pub(super) fn select_float_type(left: &Type, right: &Type) -> Option<Type> {
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

pub(super) fn is_auto_droppable(typ: &Type) -> bool {
    match typ {
        Type::I32
        | Type::I64
        | Type::F32
        | Type::F64
        | Type::IntLit
        | Type::FloatLit
        | Type::Bool
        | Type::Char
        | Type::String
        | Type::Unit
        | Type::Array(_) => true,
        Type::Linear(inner) | Type::Lazy(inner) | Type::Borrow(inner) | Type::Ref(inner) => is_auto_droppable(inner),
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
        Type::Arrow(params, ret, _requires, _throws) => {
            // Skip requires/throws: their Row entries are port/throws names, not type variables.
            for (_, typ) in params {
                collect_external_type_vars(typ, env, out);
            }
            collect_external_type_vars(ret, env, out);
        }
        Type::Ref(inner)
        | Type::Linear(inner)
        | Type::Lazy(inner)
        | Type::Borrow(inner)
        | Type::Array(inner)
        | Type::List(inner) => collect_external_type_vars(inner, env, out),
        Type::Row(throws, tail) => {
            for t in throws {
                collect_external_type_vars(t, env, out);
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
        Type::Arrow(params, ret, requires, throws) => Type::Arrow(
            params
                .iter()
                .map(|(name, typ)| (name.clone(), convert_external_type_vars(typ, vars)))
                .collect(),
            Box::new(convert_external_type_vars(ret, vars)),
            Box::new(convert_external_type_vars(requires, vars)),
            Box::new(convert_external_type_vars(throws, vars)),
        ),
        Type::Ref(inner) => Type::Ref(Box::new(convert_external_type_vars(inner, vars))),
        Type::Linear(inner) => Type::Linear(Box::new(convert_external_type_vars(inner, vars))),
        Type::Lazy(inner) => Type::Lazy(Box::new(convert_external_type_vars(inner, vars))),
        Type::Borrow(inner) => Type::Borrow(Box::new(convert_external_type_vars(inner, vars))),
        Type::Array(inner) => Type::Array(Box::new(convert_external_type_vars(inner, vars))),
        Type::List(inner) => Type::List(Box::new(convert_external_type_vars(inner, vars))),
        Type::Handler(name, req) => Type::Handler(
            name.clone(),
            Box::new(convert_external_type_vars(req, vars)),
        ),
        Type::Row(throws, tail) => Type::Row(
            throws
                .iter()
                .map(|t| convert_external_type_vars(t, vars))
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

pub(super) fn external_scheme(type_params: &[String], typ: &Type) -> Scheme {
    let vars_set: HashSet<String> = type_params.iter().cloned().collect();
    Scheme {
        typ: convert_external_type_vars(typ, &vars_set),
        vars: type_params.to_vec(),
    }
}

/// Check that all bare UserDefined names in `typ` that are not in `env.types`/`env.enums`
/// are declared in `type_params`. Returns an error listing unintroduced type variables.
pub(super) fn check_unintroduced_type_vars(
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
pub(super) fn register_stdlib_types(env: &mut TypeEnv) {
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

pub(super) fn contains_ref(t: &Type) -> bool {
    match t {
        Type::Ref(_) => true,
        Type::Arrow(p, r, req, e) => {
            contains_ref(r)
                || p.iter().any(|(_, x)| contains_ref(x))
                || contains_ref(req)
                || contains_ref(e)
        }
        Type::UserDefined(_, a) => a.iter().any(contains_ref),
        Type::Linear(i) | Type::Lazy(i) | Type::Borrow(i) | Type::Array(i) => contains_ref(i),
        Type::Row(es, t) => {
            es.iter().any(contains_ref) || t.as_ref().map_or(false, |x| contains_ref(x))
        }
        Type::Record(fs) => fs.iter().any(|(_, t)| contains_ref(t)),
        _ => false,
    }
}

pub(super) fn strip_required_port_coeffect(t: &Type, coeffect_name: &str) -> Type {
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

pub(super) fn contains_exn_throws(t: &Type) -> bool {
    contains_named_throws(t, super::THROWS_EXN)
}

/// Merges two requirement row types, deduplicating entries.
pub(super) fn merge_type_rows(base: &Type, extra: &Type) -> Type {
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
pub(super) fn extract_row_port_names(t: &Type) -> Vec<String> {
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

pub(super) fn contains_named_throws(t: &Type, effect_name: &str) -> bool {
    match t {
        Type::UserDefined(name, args) => {
            (name == effect_name && args.is_empty())
                || args
                    .iter()
                    .any(|inner| contains_named_throws(inner, effect_name))
        }
        Type::Arrow(params, ret, req, eff) => {
            params
                .iter()
                .any(|(_, p)| contains_named_throws(p, effect_name))
                || contains_named_throws(ret, effect_name)
                || contains_named_throws(req, effect_name)
                || contains_named_throws(eff, effect_name)
        }
        Type::Ref(inner) | Type::Linear(inner) | Type::Lazy(inner) | Type::Borrow(inner) | Type::Array(inner) => {
            contains_named_throws(inner, effect_name)
        }
        Type::Row(effs, tail) => {
            effs.iter().any(|e| contains_named_throws(e, effect_name))
                || tail
                    .as_ref()
                    .is_some_and(|x| contains_named_throws(x, effect_name))
        }
        Type::Record(fields) => fields
            .iter()
            .any(|(_, ft)| contains_named_throws(ft, effect_name)),
        _ => false,
    }
}

pub(super) fn is_allowed_main_throws_signature(t: &Type) -> bool {
    match t {
        Type::Unit => true,
        Type::Row(effs, tail) => tail.is_none() && effs.is_empty(),
        _ => false,
    }
}

pub(super) fn is_allowed_main_require_signature(t: &Type) -> bool {
    match t {
        Type::Unit => true,
        Type::Row(reqs, tail) => {
            tail.is_none()
                && reqs.iter().all(|r| {
                    matches!(r,
                        Type::UserDefined(name, args) if args.is_empty()
                            && is_known_runtime_perm(name)
                    )
                })
        }
        _ => false,
    }
}

fn is_known_runtime_perm(name: &str) -> bool {
    Permission::from_perm_name(name).is_some()
}

/// Check whether a statement list contains at least one `Return` statement,
/// recursively inspecting if/match/try branches. Also returns true if the
/// body ends with a tail expression (match/call/raise) that can serve as an
/// implicit return value.
pub(super) fn contains_return(body: &[Spanned<Stmt>]) -> bool {
    body.iter().any(|s| match &s.node {
        Stmt::Return(_) => true,
        Stmt::Expr(e) => expr_contains_return(e),
        Stmt::Try {
            body, catch_arms, ..
        } => {
            contains_return(body)
                || catch_arms.iter().any(|arm| contains_return(&arm.body))
        }
        Stmt::Inject { body, .. } => contains_return(body),
        _ => false,
    }) || body_ends_with_tail_expr(body)
}

/// Check whether the body ends with an expression that can serve as
/// an implicit return value (tail expression semantics).
fn body_ends_with_tail_expr(body: &[Spanned<Stmt>]) -> bool {
    body.last()
        .is_some_and(|s| matches!(&s.node, Stmt::Expr(_)))
}

fn expr_contains_return(e: &Spanned<Expr>) -> bool {
    match &e.node {
        Expr::If {
            then_branch,
            else_branch,
            ..
        } => {
            contains_return(then_branch) || else_branch.as_ref().is_some_and(|b| contains_return(b))
        }
        Expr::Match { cases, .. } => cases.iter().any(|c| contains_return(&c.body)),
        _ => false,
    }
}
