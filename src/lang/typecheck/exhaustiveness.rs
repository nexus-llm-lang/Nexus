use super::env::TypeEnv;
use super::helpers::summarize_ctor_args;
use super::unify::apply_subst_type;
use super::TypeChecker;
use crate::lang::ast::*;
use std::collections::HashMap;

impl TypeChecker {
    pub(super) fn check_exhaustiveness(
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
                // Resolve type aliases (record types) to their structural type
                if let Some(td) = env.get_type(name).cloned() {
                    if env.get_enum(name).is_none() {
                        let mut subst = HashMap::new();
                        for (p, a) in td.type_params.iter().zip(args) {
                            subst.insert(p.clone(), a.clone());
                        }
                        let fields: Vec<(String, Type)> = td
                            .fields
                            .iter()
                            .map(|(n, t)| (n.clone(), apply_subst_type(&subst, t)))
                            .collect();
                        let resolved = Type::Record(fields);
                        let mut new_types = vec![&resolved];
                        new_types.extend_from_slice(rt);
                        return self.check_matrix(env, matrix, &new_types);
                    }
                }
                let resolved_enum = env.get_enum(name).cloned().or_else(|| {
                    for cached_env in self.import_cache.values() {
                        if let Some(ed) = cached_env.enums.get(name) {
                            return Some(ed.clone());
                        }
                    }
                    for mod_env in env.modules.values() {
                        if let Some(ed) = mod_env.enums.get(name) {
                            return Some(ed.clone());
                        }
                    }
                    None
                });
                if let Some(ed) = resolved_enum {
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
                    // Strip module qualifier for qualified constructors
                    let bare_c = c.rfind('.').map_or(c.as_str(), |pos| &c[pos + 1..]);
                    if bare_c == ctor {
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
}

#[derive(Clone)]
pub(super) enum PatRef<'a> {
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
