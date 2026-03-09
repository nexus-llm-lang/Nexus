use super::env::{TypeEnv, TypeWarning};
use super::helpers::extract_row_port_names;
use super::TypeChecker;
use crate::lang::ast::*;
use std::collections::HashSet;

use super::EFFECT_EXN;

impl TypeChecker {
    pub(super) fn collect_lint_warnings(&mut self, program: &Program) {
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
            if gl.is_public || gl.name == crate::constants::ENTRYPOINT {
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

    pub(super) fn collect_unused_local_variable_warnings_in_function(
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
                Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::Return(value) => {
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
            Expr::While { cond, body } => {
                self.collect_unused_local_variable_warnings_in_expr(cond);
                self.collect_unused_local_variable_warnings_in_stmts(body);
            }
            Expr::For {
                start, end_expr, body, ..
            } => {
                self.collect_unused_local_variable_warnings_in_expr(start);
                self.collect_unused_local_variable_warnings_in_expr(end_expr);
                self.collect_unused_local_variable_warnings_in_stmts(body);
            }
            Expr::Literal(_)
            | Expr::Variable(_, _)
            | Expr::Borrow(_, _)
            | Expr::External(_, _, _) => {}
        }
    }
}

pub(super) fn find_private_type_in_public_signature(typ: &Type, env: &TypeEnv) -> Option<String> {
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

pub(super) fn extract_named_row_members(row: &Type) -> (HashSet<String>, bool) {
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

pub(super) fn collect_signature_needs_from_stmts(
    stmts: &[Spanned<Stmt>],
    env: &TypeEnv,
) -> (HashSet<String>, HashSet<String>, bool) {
    let mut reqs = HashSet::new();
    let mut effs = HashSet::new();
    let mut unknown = false;

    for stmt in stmts {
        match &stmt.node {
            Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::Return(value) => {
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
        Expr::While { cond, body } => {
            let (mut reqs, mut effs, mut unknown) = collect_signature_needs_from_expr(cond, env);
            let (body_reqs, body_effs, body_unknown) =
                collect_signature_needs_from_stmts(body, env);
            reqs.extend(body_reqs);
            effs.extend(body_effs);
            unknown |= body_unknown;
            (reqs, effs, unknown)
        }
        Expr::For {
            start, end_expr, body, ..
        } => {
            let (mut reqs, mut effs, mut unknown) = collect_signature_needs_from_expr(start, env);
            let (end_reqs, end_effs, end_unknown) =
                collect_signature_needs_from_expr(end_expr, env);
            reqs.extend(end_reqs);
            effs.extend(end_effs);
            unknown |= end_unknown;
            let (body_reqs, body_effs, body_unknown) =
                collect_signature_needs_from_stmts(body, env);
            reqs.extend(body_reqs);
            effs.extend(body_effs);
            unknown |= body_unknown;
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
        Expr::Array(items) | Expr::List(items) => {
            items.iter().any(|item| expr_mentions_name(item, target))
        }
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
        Expr::While { cond, body } => {
            expr_mentions_name(cond, target)
                || body.iter().any(|stmt| stmt_mentions_name(stmt, target))
        }
        Expr::For {
            start, end_expr, body, ..
        } => {
            expr_mentions_name(start, target)
                || expr_mentions_name(end_expr, target)
                || body.iter().any(|stmt| stmt_mentions_name(stmt, target))
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
            handlers
                .iter()
                .any(|h| h == target || h.starts_with(&format!("{}.", target)))
                || body.iter().any(|stmt| stmt_mentions_name(stmt, target))
        }
    }
}

fn collect_used_variable_keys_in_stmts(stmts: &[Spanned<Stmt>], out: &mut HashSet<String>) {
    for stmt in stmts {
        match &stmt.node {
            Stmt::Let { value, .. } | Stmt::Expr(value) | Stmt::Return(value) => {
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
        Expr::While { cond, body } => {
            collect_used_variable_keys_in_expr(cond, out);
            collect_used_variable_keys_in_stmts(body, out);
        }
        Expr::For {
            start, end_expr, body, ..
        } => {
            collect_used_variable_keys_in_expr(start, out);
            collect_used_variable_keys_in_expr(end_expr, out);
            collect_used_variable_keys_in_stmts(body, out);
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
        Expr::While { cond, body } => {
            collect_local_let_bindings_in_expr(cond, out);
            collect_local_let_bindings(body, out);
        }
        Expr::For {
            start, end_expr, body, ..
        } => {
            collect_local_let_bindings_in_expr(start, out);
            collect_local_let_bindings_in_expr(end_expr, out);
            collect_local_let_bindings(body, out);
        }
        // Nested functions/handlers are analyzed separately.
        Expr::Lambda { .. } | Expr::Handler { .. } => {}
        Expr::Literal(_) | Expr::Variable(_, _) | Expr::Borrow(_, _) | Expr::External(_, _, _) => {
        }
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
