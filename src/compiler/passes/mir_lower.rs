//! HIR → MIR lowering (static port call resolution)
//!
//! Key transformations:
//! 1. Build port method layouts from HIR ports
//! 2. Resolve all port method calls to direct calls to handler functions
//! 3. Inject statements → inline body (inject is compile-time scope, no runtime effect)

use crate::ir::hir::*;
use crate::ir::mir::*;
use crate::lang::ast::Type;
use std::collections::HashMap;

#[derive(Debug)]
pub enum MirLowerError {
    UnresolvedPort { port: String, method: String },
}

impl std::fmt::Display for MirLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MirLowerError::UnresolvedPort { port, method } => {
                write!(f, "Unresolved port method: {}.{}", port, method)
            }
        }
    }
}

/// Port method layout: ordered list of methods for each port
struct PortLayout {
    /// port_name → ordered method names
    methods: HashMap<String, Vec<String>>,
}

/// Tracks which handler is active for each port in the current scope
#[derive(Clone)]
struct HandlerScope {
    /// port_name → handler_binding_name
    active: HashMap<String, String>,
}

#[tracing::instrument(skip_all, name = "lower_hir_to_mir")]
pub fn lower_hir_to_mir(hir: &HirProgram) -> Result<MirProgram, MirLowerError> {
    let mut lowerer = MirLowerer::new(hir);
    lowerer.lower()
}

struct MirLowerer<'a> {
    hir: &'a HirProgram,
    port_layout: PortLayout,
    functions: Vec<MirFunction>,
    externals: Vec<MirExternal>,
    /// handler_binding_name → port_name
    handler_port_names: HashMap<String, String>,
    /// port_name → handler_binding_name (global resolution for functions with require clauses)
    global_port_handlers: HashMap<String, String>,
}

impl<'a> MirLowerer<'a> {
    fn new(hir: &'a HirProgram) -> Self {
        MirLowerer {
            hir,
            port_layout: PortLayout {
                methods: HashMap::new(),
            },
            functions: Vec::new(),
            externals: Vec::new(),
            handler_port_names: HashMap::new(),
            global_port_handlers: HashMap::new(),
        }
    }

    fn lower(&mut self) -> Result<MirProgram, MirLowerError> {
        // Step 1: Build port method layouts
        self.build_port_layouts();

        // Step 2: Map handler bindings to port names
        self.build_handler_port_names();

        // Step 3: Build global port→handler mapping (last handler wins for each port)
        self.build_global_port_handlers();

        // Step 4: Transform externals
        self.lower_externals();

        // Step 5: Transform functions (including synthesized handler functions)
        self.lower_functions()?;

        Ok(MirProgram {
            functions: self.functions.clone(),
            externals: self.externals.clone(),
        })
    }

    /// Step 1: For each port, record its methods in order
    fn build_port_layouts(&mut self) {
        for port in &self.hir.ports {
            let methods: Vec<String> = port.functions.iter().map(|m| m.name.clone()).collect();
            self.port_layout.methods.insert(port.name.clone(), methods);
        }
    }

    /// Step 2: For each handler binding, record its port name
    fn build_handler_port_names(&mut self) {
        for (binding_name, binding) in &self.hir.handler_bindings {
            self.handler_port_names
                .insert(binding_name.clone(), binding.port_name.clone());
        }
    }

    /// Step 3: Build global port→handler mapping. For each port, the last registered
    /// handler binding is used as the default for functions with require clauses.
    fn build_global_port_handlers(&mut self) {
        for (binding_name, port_name) in &self.handler_port_names {
            self.global_port_handlers
                .insert(port_name.clone(), binding_name.clone());
        }
    }

    /// Look up the return type of a function by name
    fn lookup_ret_type(&self, func_name: &str) -> Type {
        for f in &self.hir.functions {
            if f.name == func_name {
                return f.ret_type.clone();
            }
        }
        for binding in self.hir.handler_bindings.values() {
            for f in &binding.functions {
                if f.name == func_name {
                    return f.ret_type.clone();
                }
            }
        }
        for ext in &self.hir.externals {
            if ext.name == func_name {
                return ext.ret_type.clone();
            }
        }
        Type::I64
    }

    fn lower_externals(&mut self) {
        for ext in &self.hir.externals {
            self.externals.push(MirExternal {
                name: ext.name.clone(),
                wasm_module: ext.wasm_module.clone(),
                wasm_name: ext.wasm_name.clone(),
                params: ext
                    .params
                    .iter()
                    .map(|p| MirParam {
                        label: p.label.clone(),
                        name: p.name.clone(),
                        typ: p.typ.clone(),
                    })
                    .collect(),
                ret_type: ext.ret_type.clone(),
                effects: ext.effects.clone(),
            });
        }
    }

    fn lower_functions(&mut self) -> Result<(), MirLowerError> {
        // Create initial scope from global port handlers (for functions with require clauses)
        let global_scope = HandlerScope {
            active: self.global_port_handlers.clone(),
        };

        for func in &self.hir.functions {
            let mir_func = self.lower_function(func, &global_scope)?;
            self.functions.push(mir_func);
        }

        // Also lower handler method functions as regular MIR functions
        for (_binding_name, binding) in &self.hir.handler_bindings {
            for func in &binding.functions {
                let mir_func = self.lower_function(func, &global_scope)?;
                self.functions.push(mir_func);
            }
        }

        Ok(())
    }

    fn lower_function(
        &self,
        func: &HirFunction,
        scope: &HandlerScope,
    ) -> Result<MirFunction, MirLowerError> {
        let params: Vec<MirParam> = func
            .params
            .iter()
            .map(|p| MirParam {
                label: p.label.clone(),
                name: p.name.clone(),
                typ: p.typ.clone(),
            })
            .collect();

        let body = self.lower_stmts(&func.body, scope)?;

        Ok(MirFunction {
            name: func.name.clone(),
            params,
            ret_type: func.ret_type.clone(),
            body,
        })
    }

    fn lower_stmts(
        &self,
        stmts: &[HirStmt],
        scope: &HandlerScope,
    ) -> Result<Vec<MirStmt>, MirLowerError> {
        let mut result = Vec::new();
        for stmt in stmts {
            match self.lower_stmt(stmt, scope)? {
                InjectResult::Single(s) => result.push(s),
                InjectResult::Multiple(stmts) => result.extend(stmts),
            }
        }
        Ok(result)
    }

    fn lower_stmt(
        &self,
        stmt: &HirStmt,
        scope: &HandlerScope,
    ) -> Result<InjectResult, MirLowerError> {
        match stmt {
            HirStmt::Let { name, typ, value } => Ok(InjectResult::Single(MirStmt::Let {
                name: name.clone(),
                typ: typ.clone().unwrap_or(Type::Unit),
                expr: self.lower_expr(value, scope)?,
            })),
            HirStmt::Expr(expr) => Ok(InjectResult::Single(MirStmt::Expr(
                self.lower_expr(expr, scope)?,
            ))),
            HirStmt::Return(expr) => Ok(InjectResult::Single(MirStmt::Return(
                self.lower_expr(expr, scope)?,
            ))),
            HirStmt::Assign { target, value } => Ok(InjectResult::Single(MirStmt::Assign {
                target: self.lower_expr(target, scope)?,
                value: self.lower_expr(value, scope)?,
            })),
            HirStmt::Conc(tasks) => {
                let mir_tasks: Vec<MirFunction> = tasks
                    .iter()
                    .map(|t| self.lower_function(t, scope))
                    .collect::<Result<_, _>>()?;
                Ok(InjectResult::Single(MirStmt::Conc(mir_tasks)))
            }
            HirStmt::Try {
                body,
                catch_param,
                catch_body,
            } => Ok(InjectResult::Single(MirStmt::Try {
                body: self.lower_stmts(body, scope)?,
                catch_param: catch_param.clone(),
                catch_body: self.lower_stmts(catch_body, scope)?,
            })),
            HirStmt::Inject { handlers, body } => {
                // Inject activates handlers: create new scope with handler bindings
                let mut new_scope = scope.clone();
                for handler_name in handlers {
                    if let Some(port_name) = self.handler_port_names.get(handler_name) {
                        new_scope
                            .active
                            .insert(port_name.clone(), handler_name.clone());
                    }
                }
                // Inline the inject body — inject has no runtime effect
                let mir_body = self.lower_stmts(body, &new_scope)?;
                Ok(InjectResult::Multiple(mir_body))
            }
        }
    }

    fn lower_expr(&self, expr: &HirExpr, scope: &HandlerScope) -> Result<MirExpr, MirLowerError> {
        match expr {
            HirExpr::Literal(lit) => Ok(MirExpr::Literal(lit.clone())),
            HirExpr::Variable(name, _sigil) => Ok(MirExpr::Variable(name.clone())),
            HirExpr::BinaryOp(lhs, op, rhs) => Ok(MirExpr::BinaryOp(
                Box::new(self.lower_expr(lhs, scope)?),
                *op,
                Box::new(self.lower_expr(rhs, scope)?),
            )),
            HirExpr::Borrow(name, _sigil) => Ok(MirExpr::Borrow(name.clone())),
            HirExpr::Call { func, args } => {
                // Check if this is a port-qualified call (e.g., "Logger.info")
                if let Some((port_name, method_name)) = func.split_once('.') {
                    if self.port_layout.methods.contains_key(port_name) {
                        return self.lower_port_call(port_name, method_name, args, scope);
                    }
                }

                let mir_args: Vec<(String, MirExpr)> = args
                    .iter()
                    .map(|(label, expr)| Ok((label.clone(), self.lower_expr(expr, scope)?)))
                    .collect::<Result<_, MirLowerError>>()?;

                let ret_type = self.lookup_ret_type(func);

                Ok(MirExpr::Call {
                    func: func.clone(),
                    args: mir_args,

                    ret_type,
                })
            }
            HirExpr::Constructor { variant, args } => {
                let mir_args: Vec<MirExpr> = args
                    .iter()
                    .map(|e| self.lower_expr(e, scope))
                    .collect::<Result<_, _>>()?;
                Ok(MirExpr::Constructor {
                    name: variant.clone(),
                    args: mir_args,
                })
            }
            HirExpr::Record(fields) => {
                let mir_fields: Vec<(String, MirExpr)> = fields
                    .iter()
                    .map(|(name, expr)| Ok((name.clone(), self.lower_expr(expr, scope)?)))
                    .collect::<Result<_, MirLowerError>>()?;
                Ok(MirExpr::Record(mir_fields))
            }
            HirExpr::Array(items) => {
                let mir_items: Vec<MirExpr> = items
                    .iter()
                    .map(|e| self.lower_expr(e, scope))
                    .collect::<Result<_, _>>()?;
                Ok(MirExpr::Array(mir_items))
            }
            HirExpr::Index(arr, idx) => Ok(MirExpr::Index(
                Box::new(self.lower_expr(arr, scope)?),
                Box::new(self.lower_expr(idx, scope)?),
            )),
            HirExpr::FieldAccess(expr, field) => Ok(MirExpr::FieldAccess(
                Box::new(self.lower_expr(expr, scope)?),
                field.clone(),
            )),
            HirExpr::If {
                cond,
                then_branch,
                else_branch,
            } => Ok(MirExpr::If {
                cond: Box::new(self.lower_expr(cond, scope)?),
                then_body: self.lower_stmts(then_branch, scope)?,
                else_body: else_branch
                    .as_ref()
                    .map(|b| self.lower_stmts(b, scope))
                    .transpose()?,
            }),
            HirExpr::Match { target, cases } => {
                let mir_cases: Vec<MirMatchCase> = cases
                    .iter()
                    .map(|c| {
                        Ok(MirMatchCase {
                            pattern: self.lower_pattern(&c.pattern),
                            body: self.lower_stmts(&c.body, scope)?,
                        })
                    })
                    .collect::<Result<_, MirLowerError>>()?;
                Ok(MirExpr::Match {
                    target: Box::new(self.lower_expr(target, scope)?),
                    cases: mir_cases,
                })
            }
            HirExpr::Lambda {
                params: _,
                ret_type: _,
                body: _,
            } => {
                // Lambda in expression position — should have been lifted to top-level
                // by the time we reach MIR lowering. Emit unit as placeholder.
                Ok(MirExpr::Literal(crate::lang::ast::Literal::Unit))
            }
            HirExpr::Raise(expr) => Ok(MirExpr::Raise(Box::new(self.lower_expr(expr, scope)?))),
            HirExpr::External(sym, _tparams, _typ) => Ok(MirExpr::Variable(sym.clone())),
            HirExpr::Handler { functions: _ } => {
                // Handler expressions collected into handler_bindings during HIR build
                Ok(MirExpr::Literal(crate::lang::ast::Literal::Unit))
            }
        }
    }

    /// Resolve a port method call to a direct call to the handler function.
    /// Uses the handler scope to find which handler provides this port.
    fn lower_port_call(
        &self,
        port_name: &str,
        method_name: &str,
        args: &[(String, HirExpr)],
        scope: &HandlerScope,
    ) -> Result<MirExpr, MirLowerError> {
        // Find which handler binding provides this port
        let handler_name =
            scope
                .active
                .get(port_name)
                .ok_or_else(|| MirLowerError::UnresolvedPort {
                    port: port_name.to_string(),
                    method: method_name.to_string(),
                })?;

        // Resolve to the handler's synthesized function
        let func_name = format!("__handler_{}_{}", handler_name, method_name);
        let ret_type = self.lookup_ret_type(&func_name);

        let mir_args: Vec<(String, MirExpr)> = args
            .iter()
            .map(|(label, expr)| Ok((label.clone(), self.lower_expr(expr, scope)?)))
            .collect::<Result<_, MirLowerError>>()?;

        Ok(MirExpr::Call {
            func: func_name,
            args: mir_args,
            ret_type,
        })
    }

    fn lower_pattern(&self, pattern: &HirPattern) -> MirPattern {
        match pattern {
            HirPattern::Literal(lit) => MirPattern::Literal(lit.clone()),
            HirPattern::Variable(name, sigil) => MirPattern::Variable(name.clone(), sigil.clone()),
            HirPattern::Constructor { variant, fields } => MirPattern::Constructor {
                name: variant.clone(),
                fields: fields
                    .iter()
                    .map(|(label, p)| (label.clone(), self.lower_pattern(p)))
                    .collect(),
            },
            HirPattern::Record(fields, open) => MirPattern::Record(
                fields
                    .iter()
                    .map(|(name, p)| (name.clone(), self.lower_pattern(p)))
                    .collect(),
                *open,
            ),
            HirPattern::Wildcard => MirPattern::Wildcard,
        }
    }
}

/// Result of lowering a statement — inject produces multiple statements
enum InjectResult {
    Single(MirStmt),
    Multiple(Vec<MirStmt>),
}
