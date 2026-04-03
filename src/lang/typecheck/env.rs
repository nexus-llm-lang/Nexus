use super::helpers::is_auto_droppable;
use super::unify::apply_subst_type;
use super::Subst;
use crate::lang::ast::*;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct TypeError {
    pub message: String,
    pub span: Span,
    /// Additional labeled spans for multi-location diagnostics (e.g. "expected X here, found Y there").
    pub labels: Vec<(Span, String)>,
}

impl TypeError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        TypeError {
            message: message.into(),
            span,
            labels: Vec::new(),
        }
    }

    pub fn with_labels(mut self, labels: Vec<(Span, String)>) -> Self {
        self.labels = labels;
        self
    }
}

#[derive(Debug, Clone)]
pub struct TypeWarning {
    pub message: String,
    pub span: Span,
}

impl From<(String, Span)> for TypeError {
    fn from((message, span): (String, Span)) -> Self {
        TypeError::new(message, span)
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
        let unused: Vec<_> = self
            .linear_vars
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
            return Err(TypeError::new(
                format!("Unused linear: {:?}", unused),
                span.clone(),
            ));
        }
        Ok(())
    }

    fn contains_linear_type_inner(&self, typ: &Type, visiting: &mut HashSet<String>) -> bool {
        match typ {
            Type::Linear(_) | Type::Lazy(_) | Type::Array(_) => true,
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
    /// For linear references (`%name`), also tries the unsigiled name (`name`)
    /// since `fn (x: %Type)` stores the parameter as `x`, not `%x`.
    pub fn get(&self, name: &str) -> Option<&Scheme> {
        if let Some(scheme) = self.vars.get(name) {
            return Some(scheme);
        }
        // %name → try name (linear param stored without sigil prefix)
        if let Some(stripped) = name.strip_prefix('%') {
            if let Some(scheme) = self.vars.get(stripped) {
                if matches!(&scheme.typ, crate::types::Type::Linear(_) | crate::types::Type::Lazy(_)) {
                    return Some(scheme);
                }
            }
        }
        // name → try %name (linear let-binding stored with sigil prefix)
        if !name.starts_with('%') && !name.starts_with('&') && !name.starts_with('~') {
            let linear_key = format!("%{}", name);
            if let Some(scheme) = self.vars.get(&linear_key) {
                if matches!(&scheme.typ, crate::types::Type::Linear(_) | crate::types::Type::Lazy(_)) {
                    return Some(scheme);
                }
            }
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
    /// For `%name`, also checks unsigiled `name` (linear param stored without prefix).
    pub fn consume(&mut self, name: &str) -> Result<(), String> {
        if self.linear_vars.remove(name) {
            return Ok(());
        }
        // %name → try name (linear param stored without sigil prefix)
        if let Some(stripped) = name.strip_prefix('%') {
            if self.linear_vars.remove(stripped) {
                return Ok(());
            }
        }
        let check_name = if self.vars.contains_key(name) {
            name.to_string()
        } else if let Some(stripped) = name.strip_prefix('%') {
            if self.vars.contains_key(stripped) {
                stripped.to_string()
            } else {
                return Err(format!("Variable {} not found", name));
            }
        } else {
            return Err(format!("Variable {} not found", name));
        };
        if let Some(s) = self.vars.get(&check_name) {
            if self.contains_linear_type(&s.typ) {
                return Err(format!("Linear variable {} already consumed", name));
            }
        }
        Ok(())
    }
}
