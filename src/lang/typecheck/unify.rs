use super::env::{Scheme, TypeEnv};
use super::{Subst, TypeChecker};
use crate::lang::ast::*;
use std::collections::{HashMap, HashSet};

impl TypeChecker {
    pub(super) fn generalize(&self, env: &TypeEnv, typ: Type) -> Scheme {
        let evs = get_free_vars_env(env);
        let tvs = get_free_vars_type(&typ);
        let free: Vec<String> = tvs.difference(&evs).cloned().collect();
        Scheme { vars: free, typ }
    }

    pub(super) fn unify(&mut self, t1: &Type, t2: &Type) -> Result<Subst, String> {
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
            (Type::Handler(a, req_a), Type::Handler(b, req_b)) if a == b => {
                self.unify(req_a, req_b)
            }
            // Expand named record (TypeDef) to Record for structural unification
            (Type::UserDefined(name, args), Type::Record(_))
            | (Type::Record(_), Type::UserDefined(name, args)) => {
                if let Some(td) = self.type_defs.get(name).cloned() {
                    let mut su = HashMap::new();
                    for (param, arg) in td.type_params.iter().zip(args) {
                        su.insert(param.clone(), arg.clone());
                    }
                    let expanded = Type::Record(
                        td.fields
                            .iter()
                            .map(|(n, t)| (n.clone(), apply_subst_type(&su, t)))
                            .collect(),
                    );
                    // Unify both sides against the expanded record form
                    let other = match t1 {
                        Type::Record(_) => t1,
                        _ => t2,
                    };
                    self.unify(other, &expanded)
                } else {
                    Err(format!("Mismatch: {} vs {}", t1, t2))
                }
            }
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
        Type::Handler(name, req) => {
            Type::Handler(name.clone(), Box::new(apply_subst_type(subst, req)))
        }
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

pub(super) fn compose_subst(s1: &Subst, s2: &Subst) -> Subst {
    let mut res = s2.clone();
    for (k, v) in s1 {
        res.insert(k.clone(), apply_subst_type(s2, v));
    }
    res
}

pub(super) fn get_free_vars_type(typ: &Type) -> HashSet<String> {
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
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::Array(i) | Type::List(i) => {
            get_free_vars_type(i)
        }
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

pub(super) fn get_free_vars_env(env: &TypeEnv) -> HashSet<String> {
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
        Type::Ref(i) | Type::Linear(i) | Type::Borrow(i) | Type::Array(i) | Type::List(i) => {
            occurs_check(n, i)
        }
        Type::Row(es, t) => {
            es.iter().any(|x| occurs_check(n, x))
                || t.as_ref().map_or(false, |x| occurs_check(n, x))
        }
        Type::Record(fs) => fs.iter().any(|(_, t)| occurs_check(n, t)),
        _ => false,
    }
}
