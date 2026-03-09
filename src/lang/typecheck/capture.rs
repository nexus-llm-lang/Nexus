use crate::lang::ast::*;
use std::collections::HashSet;

pub(super) fn lambda_references_name(body: &[Spanned<Stmt>], params: &[Param], name: &str) -> bool {
    let mut outer_keys = HashSet::new();
    outer_keys.insert(name.to_string());
    collect_lambda_captures(body, params, &outer_keys).contains(name)
}

pub(super) fn collect_lambda_captures(
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
        Expr::While { cond, body } => {
            collect_expr_captures(cond, outer_keys, bound_keys, bound_call_names, captures);
            collect_stmt_captures(body, outer_keys, bound_keys, bound_call_names, captures);
        }
        Expr::For {
            var,
            start,
            end_expr,
            body,
        } => {
            collect_expr_captures(start, outer_keys, bound_keys, bound_call_names, captures);
            collect_expr_captures(end_expr, outer_keys, bound_keys, bound_call_names, captures);
            let mut for_bound_keys = bound_keys.clone();
            for_bound_keys.insert(var.clone());
            collect_stmt_captures(
                body,
                outer_keys,
                &for_bound_keys,
                bound_call_names,
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
