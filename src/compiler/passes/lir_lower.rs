//! MIR → LIR (ANF conversion)
//!
//! Flattens complex MIR expressions into ANF form:
//! - All operands become atoms (variables or literals)
//! - Complex expressions are extracted into let-bound temporaries
//! - If/Match compiled into IfReturn chains

use crate::compiler::type_tag::constructor_tag;
use crate::intern::Symbol;
use crate::ir::lir::*;
use crate::ir::mir::*;
use crate::types::{BinaryOp, EnumDef, Literal, Span, Type, WasmRepr};
use std::collections::{HashMap, HashSet};

#[derive(Debug)]
pub enum LirLowerError {
    UnsupportedExpression { detail: String, span: Span },
    FunctionMayNotReturn { name: String, span: Span },
    UnresolvedType { detail: String, span: Span },
}

impl std::fmt::Display for LirLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LirLowerError::UnsupportedExpression { detail, .. } => {
                write!(f, "Unsupported expression in LIR lowering: {}", detail)
            }
            LirLowerError::FunctionMayNotReturn { name, .. } => {
                write!(f, "Function '{}' may not return a value", name)
            }
            LirLowerError::UnresolvedType { detail, .. } => {
                write!(f, "Unresolved type in LIR lowering: {}", detail)
            }
        }
    }
}

impl LirLowerError {
    pub fn span(&self) -> &Span {
        match self {
            LirLowerError::UnsupportedExpression { span, .. }
            | LirLowerError::FunctionMayNotReturn { span, .. }
            | LirLowerError::UnresolvedType { span, .. } => span,
        }
    }
}

#[tracing::instrument(skip_all, name = "lower_mir_to_lir")]
pub fn lower_mir_to_lir(
    mir: &MirProgram,
    enum_defs: &[EnumDef],
) -> Result<LirProgram, LirLowerError> {
    let mut lowerer = LirLowerer::new(mir, enum_defs);
    lowerer.lower()
}

struct LirLowerer<'a> {
    mir: &'a MirProgram,
    enum_defs: &'a [EnumDef],
    task_functions: Vec<LirFunction>,
}

impl<'a> LirLowerer<'a> {
    fn new(mir: &'a MirProgram, enum_defs: &'a [EnumDef]) -> Self {
        LirLowerer {
            mir,
            enum_defs,
            task_functions: Vec::new(),
        }
    }

    fn lower(&mut self) -> Result<LirProgram, LirLowerError> {
        let mut functions = Vec::new();
        for func in &self.mir.functions {
            let (lir_func, task_fns) = lower_mir_function(func, self.enum_defs)?;
            functions.push(lir_func);
            self.task_functions.extend(task_fns);
        }
        functions.append(&mut self.task_functions);

        let externals = self
            .mir
            .externals
            .iter()
            .map(|ext| LirExternal {
                name: ext.name.clone(),
                wasm_module: ext.wasm_module.clone(),
                wasm_name: ext.wasm_name.clone(),
                params: ext
                    .params
                    .iter()
                    .map(|p| LirParam {
                        label: p.label.clone(),
                        name: p.name.clone(),
                        typ: p.typ.clone(),
                    })
                    .collect(),
                ret_type: ext.ret_type.clone(),
                throws: ext.throws.clone(),
            })
            .collect();

        // Closure conversion: transform funcref-target functions for uniform closure calling convention
        closure_convert(&mut functions, &self.mir.functions)?;

        Ok(LirProgram {
            functions,
            externals,
        })
    }
}

/// Lower a single MIR function to LIR, returning the function and any task functions
/// accumulated from conc blocks within its body.
fn lower_mir_function(
    func: &MirFunction,
    enum_defs: &[EnumDef],
) -> Result<(LirFunction, Vec<LirFunction>), LirLowerError> {
    let mut ctx = LowerCtx::new(enum_defs, func.source_file.clone(), func.source_line);

    // Register params in vars (both wasm and semantic types)
    for p in &func.params {
        ctx.vars.insert(p.name.clone(), wasm_type(&p.typ));
        ctx.semantic_vars.insert(p.name.clone(), p.typ.clone());
    }
    // Lower body
    let mut ret_atom = None;
    for stmt in &func.body {
        if let Some(atom) = ctx.lower_stmt(stmt, &func.ret_type)? {
            ret_atom = Some(atom);
            break;
        }
    }

    // Determine return value
    let ret = if let Some(ret) = ret_atom {
        ret
    } else if let Some(ret) = fallback_return_atom_from_terminal_stmt(&ctx.stmts) {
        ret
    } else if matches!(func.ret_type, Type::Unit) {
        LirAtom::Unit
    } else {
        // Function body doesn't explicitly return a value. This happens when
        // a match in tail position has only side-effect cases (then_ret=None).
        // Use a default placeholder — the actual returns are handled by
        // IfReturn statements within the body (nested if/match with returns).
        default_atom_for_type(&wasm_type(&func.ret_type))
    };

    let mut params: Vec<LirParam> = func
        .params
        .iter()
        .map(|p| LirParam {
            label: p.label.clone(),
            name: p.name.clone(),
            typ: p.typ.clone(),
        })
        .collect();
    params.sort_by(|a, b| a.label.cmp(&b.label));

    let task_functions = ctx.task_functions;

    Ok((
        LirFunction {
            name: func.name.clone(),
            params,
            ret_type: func.ret_type.clone(),
            requires: Type::Row(Vec::new(), None),
            throws: Type::Row(Vec::new(), None),
            body: ctx.stmts,
            ret,
            span: func.span.clone(),
            source_file: func.source_file.clone(),
            source_line: func.source_line,
        },
        task_functions,
    ))
}

/// Context for lowering a single function body
struct LowerCtx<'a> {
    vars: HashMap<Symbol, Type>,
    /// Semantic types for variables (pre-wasm-lowering) — used for field access resolution
    semantic_vars: HashMap<Symbol, Type>,
    stmts: Vec<LirStmt>,
    temp_counter: usize,
    /// Task functions lifted from conc blocks
    task_functions: Vec<LirFunction>,
    enum_defs: &'a [EnumDef],
    source_file: Option<String>,
    source_line: Option<u32>,
}

impl<'a> LowerCtx<'a> {
    fn new(enum_defs: &'a [EnumDef], source_file: Option<String>, source_line: Option<u32>) -> Self {
        LowerCtx {
            vars: HashMap::new(),
            semantic_vars: HashMap::new(),
            stmts: Vec::new(),
            temp_counter: 0,
            task_functions: Vec::new(),
            enum_defs,
            source_file,
            source_line,
        }
    }

    fn new_temp(&mut self) -> Symbol {
        let name = Symbol::from(format!("__t{}", self.temp_counter));
        self.temp_counter += 1;
        name
    }

    /// Bind a complex expression to a temporary variable, returning an atom reference
    fn bind_expr_to_temp(&mut self, expr: LirExpr, typ: Type) -> LirAtom {
        let name = self.new_temp();
        self.vars.insert(name, typ.clone());
        self.stmts.push(LirStmt::Let {
            name,
            typ: typ.clone(),
            expr,
        });
        LirAtom::Var { name, typ }
    }

    /// Like bind_expr_to_temp but also registers the semantic type for pattern matching.
    fn bind_expr_to_temp_semantic(
        &mut self,
        expr: LirExpr,
        typ: Type,
        semantic_type: Type,
    ) -> LirAtom {
        let name = self.new_temp();
        self.vars.insert(name, typ.clone());
        self.semantic_vars.insert(name, semantic_type);
        self.stmts.push(LirStmt::Let {
            name,
            typ: typ.clone(),
            expr,
        });
        LirAtom::Var { name, typ }
    }

    /// Lower a MIR statement, optionally returning an atom for explicit returns
    fn lower_stmt(
        &mut self,
        stmt: &MirStmt,
        ret_type: &Type,
    ) -> Result<Option<LirAtom>, LirLowerError> {
        match stmt {
            MirStmt::Let { name, typ, expr } => {
                // Infer semantic type from expression BEFORE lowering
                // (MIR typ is often None/Unit for unannotated lets)
                let pre_semantic_type = {
                    let expr_inferred = self.infer_semantic_type(expr);
                    if matches!(typ, Type::Unit) || matches!(typ, Type::I64) {
                        expr_inferred
                    } else {
                        typ.clone()
                    }
                };
                let atom = self.lower_expr_to_atom(expr)?;
                let inferred = atom.typ();
                // After lowering, the result atom (if a temp var from match/if expr)
                // may have a more accurate semantic type set by lower_match_expr/lower_if_expr.
                // Prefer that over the pre-lowering inference which can't resolve pattern bindings.
                let semantic_type = match &atom {
                    LirAtom::Var {
                        name: var_name, ..
                    } => self
                        .semantic_vars
                        .get(var_name)
                        .cloned()
                        .unwrap_or(pre_semantic_type),
                    _ => pre_semantic_type,
                };
                self.vars.insert(name.clone(), inferred.clone());
                self.semantic_vars.insert(name.clone(), semantic_type);
                self.stmts.push(LirStmt::Let {
                    name: name.clone(),
                    typ: inferred,
                    expr: LirExpr::Atom(atom),
                });
                Ok(None)
            }
            MirStmt::Expr(expr) => match expr {
                MirExpr::If {
                    cond,
                    then_body,
                    else_body,
                } => {
                    self.lower_if_stmt(cond, then_body, else_body.as_deref(), ret_type)?;
                    Ok(None)
                }
                MirExpr::Match { target, cases } => {
                    self.lower_match_stmt(target, cases, ret_type)?;
                    Ok(None)
                }
                MirExpr::While { cond, body } => {
                    self.lower_while_stmt(cond, body, ret_type)?;
                    Ok(None)
                }
                _ => {
                    let _atom = self.lower_expr_to_atom(expr)?;
                    Ok(None)
                }
            },
            MirStmt::Return(expr) => {
                if let MirExpr::Call {
                    func,
                    args,
                    ret_type,
                } = expr
                {
                    let mut lir_args = Vec::new();
                    for (label, e) in args {
                        let atom = self.lower_expr_to_atom(e)?;
                        lir_args.push((label.clone(), atom));
                    }
                    lir_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                    let typ = wasm_type(ret_type);
                    let atom = self.bind_expr_to_temp(
                        LirExpr::TailCall {
                            func: func.clone(),
                            args: lir_args,
                            typ: typ.clone(),
                        },
                        typ,
                    );
                    return Ok(Some(atom));
                }
                let atom = self.lower_expr_to_atom(expr)?;
                Ok(Some(atom))
            }
            MirStmt::Assign { target, value } => {
                // Lower both sides to atoms
                let target_atom = self.lower_expr_to_atom(target)?;
                let value_atom = self.lower_expr_to_atom(value)?;
                // Emit as a let re-binding (ANF doesn't have mutation)
                if let LirAtom::Var { name, typ } = target_atom {
                    self.stmts.push(LirStmt::Let {
                        name,
                        typ,
                        expr: LirExpr::Atom(value_atom),
                    });
                }
                Ok(None)
            }
            MirStmt::Conc(tasks) => {
                let mut task_refs = Vec::new();
                for task in tasks {
                    let free_vars = collect_free_vars(&task.body);
                    let task_name = Symbol::from(format!("__conc_{}", task.name));
                    let capture_params: Vec<MirParam> = free_vars
                        .iter()
                        .map(|&name| {
                            let typ = self.semantic_vars.get(&name).cloned()
                                .ok_or_else(|| LirLowerError::UnresolvedType {
                                    detail: format!("conc capture variable '{}' not in semantic_vars", name),
                                    span: task.span.clone(),
                                })?;
                            Ok(MirParam {
                                label: name,
                                name,
                                typ,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    let task_mir = MirFunction {
                        name: task_name,
                        params: capture_params,
                        ret_type: Type::Unit,
                        body: task.body.clone(),
                        span: task.span.clone(),
                        source_file: self.source_file.clone(),
                        source_line: self.source_line,
                        captures: vec![],
                    };
                    let (lir_func, nested_tasks) = lower_mir_function(&task_mir, self.enum_defs)?;
                    self.task_functions.push(lir_func);
                    self.task_functions.extend(nested_tasks);
                    let args: Vec<(Symbol, LirAtom)> = free_vars
                        .iter()
                        .map(|&name| {
                            let typ = self.vars.get(&name).cloned()
                                .ok_or_else(|| LirLowerError::UnresolvedType {
                                    detail: format!("conc capture variable '{}' not in vars", name),
                                    span: task.span.clone(),
                                })?;
                            Ok((
                                name,
                                LirAtom::Var {
                                    name,
                                    typ,
                                },
                            ))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    task_refs.push(ConcTask {
                        func_name: task_name,
                        args,
                    });
                }
                self.stmts.push(LirStmt::Conc { tasks: task_refs });
                Ok(None)
            }
            MirStmt::Try {
                body,
                catch_param,
                catch_body,
            } => {
                let (body_stmts, body_ret) = self.lower_block(body, ret_type)?;
                let mut catch_vars = self.vars.clone();
                catch_vars.insert(catch_param.clone(), Type::String);
                let mut catch_sem_vars = self.semantic_vars.clone();
                catch_sem_vars.insert(catch_param.clone(), Type::String);
                let (catch_stmts, catch_ret) =
                    self.lower_block_with_vars(catch_body, ret_type, catch_vars, catch_sem_vars)?;

                self.stmts.push(LirStmt::TryCatch {
                    body: body_stmts,
                    body_ret,
                    catch_param: catch_param.clone(),
                    catch_param_typ: Type::String,
                    catch_body: catch_stmts,
                    catch_ret,
                });
                Ok(None)
            }
        }
    }

    /// Lower an if statement (possibly producing IfReturn)
    fn lower_if_stmt(
        &mut self,
        cond: &MirExpr,
        then_body: &[MirStmt],
        else_body: Option<&[MirStmt]>,
        ret_type: &Type,
    ) -> Result<(), LirLowerError> {
        let cond_atom = self.lower_expr_to_atom(cond)?;
        let (then_stmts, then_ret) = self.lower_block(then_body, ret_type)?;

        if let Some(else_body) = else_body {
            let (else_stmts, else_ret) = self.lower_block(else_body, ret_type)?;

            if then_ret.is_some() || else_ret.is_some() {
                self.stmts.push(LirStmt::IfReturn {
                    cond: cond_atom,
                    then_body: then_stmts,
                    then_ret: Some(then_ret.unwrap_or_else(|| default_atom_for_type(ret_type))),
                    else_body: else_stmts,
                    else_ret,
                    ret_type: ret_type.clone(),
                });
            } else {
                self.stmts.push(LirStmt::If {
                    cond: cond_atom,
                    then_body: then_stmts,
                    else_body: else_stmts,
                });
            }
        } else if then_ret.is_some() {
            // then branch returns — need IfReturn even without else
            self.stmts.push(LirStmt::IfReturn {
                cond: cond_atom,
                then_body: then_stmts,
                then_ret: Some(then_ret.unwrap_or_else(|| default_atom_for_type(ret_type))),
                else_body: Vec::new(),
                else_ret: None,
                ret_type: ret_type.clone(),
            });
        } else {
            self.stmts.push(LirStmt::If {
                cond: cond_atom,
                then_body: then_stmts,
                else_body: Vec::new(),
            });
        }
        Ok(())
    }

    /// Lower an if expression in atom position (as a value).
    /// Creates a temp variable, assigns in each branch, returns the temp.
    fn lower_if_expr(
        &mut self,
        cond: &MirExpr,
        then_body: &[MirStmt],
        else_body: Option<&[MirStmt]>,
    ) -> Result<LirAtom, LirLowerError> {
        let cond_atom = self.lower_expr_to_atom(cond)?;

        // Infer result type from the then branch's last statement
        let semantic_result_type = if let Some(last) = then_body.last() {
            self.infer_stmt_type(last)
        } else {
            Type::Unit
        };
        let result_type = wasm_type(&semantic_result_type);

        let result_name = self.new_temp();
        self.vars.insert(result_name.clone(), result_type.clone());
        self.semantic_vars
            .insert(result_name.clone(), semantic_result_type);

        // Initialize with placeholder
        self.stmts.push(LirStmt::Let {
            name: result_name.clone(),
            typ: result_type.clone(),
            expr: LirExpr::Atom(default_atom_for_type(&result_type)),
        });

        // Lower branches directly — do NOT use promote_last_expr_to_return, which
        // creates synthetic Return stmts that trigger TailCall optimization and
        // incorrectly return from the enclosing function instead of just producing
        // a value for the if-expression.
        let then_stmts =
            self.lower_branch_for_value(then_body, result_name, &result_type)?;

        let else_stmts = if let Some(else_body) = else_body {
            self.lower_branch_for_value(else_body, result_name, &result_type)?
        } else {
            Vec::new()
        };

        self.stmts.push(LirStmt::If {
            cond: cond_atom,
            then_body: then_stmts,
            else_body: else_stmts,
        });

        Ok(LirAtom::Var {
            name: result_name,
            typ: result_type,
        })
    }

    /// Lower a branch body (then/else) for value extraction without synthetic Returns.
    /// Lowers all statements, then assigns the last expression's value to `result_name`.
    fn lower_branch_for_value(
        &mut self,
        body: &[MirStmt],
        result_name: Symbol,
        result_type: &Type,
    ) -> Result<Vec<LirStmt>, LirLowerError> {
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = self.vars.clone();
        let saved_semantic_vars = self.semantic_vars.clone();

        if !body.is_empty() {
            let last_idx = body.len() - 1;
            for stmt in &body[..last_idx] {
                self.lower_stmt(stmt, &Type::Unit)?;
            }
            // Extract the value from the last statement
            let value = match &body[last_idx] {
                MirStmt::Expr(expr) => Some(self.lower_expr_to_atom(expr)?),
                MirStmt::Return(expr) => Some(self.lower_expr_to_atom(expr)?),
                _ => {
                    self.lower_stmt(&body[last_idx], &Type::Unit)?;
                    None
                }
            };
            if let Some(val) = value {
                self.stmts.push(LirStmt::Let {
                    name: result_name,
                    typ: result_type.clone(),
                    expr: LirExpr::Atom(val),
                });
            }
        }

        let branch_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
        self.vars = saved_vars;
        self.semantic_vars = saved_semantic_vars;
        Ok(branch_stmts)
    }

    /// Lower a match statement into a chain of If/IfReturn statements.
    /// Each case independently uses IfReturn (when it genuinely returns a
    /// value) or plain If (side-effect-only body). This prevents side-effect
    /// cases from emitting spurious WASM `return` instructions that would
    /// make code after the match unreachable.
    fn lower_match_stmt(
        &mut self,
        target: &MirExpr,
        cases: &[MirMatchCase],
        ret_type: &Type,
    ) -> Result<(), LirLowerError> {
        let target_atom = self.lower_expr_to_atom(target)?;

        if cases.is_empty() {
            return Ok(());
        }

        // Build chain: each case becomes an IfReturn.
        // Per-case decision: if the case has a genuine return value (from
        // a bare expression, explicit return, or nested IfReturn/TryCatch),
        // then_ret is Some and codegen emits WASM `return`. Otherwise,
        // then_ret is None and the case body executes for side effects
        // only — no WASM `return` is emitted, so code after the match
        // is reachable.
        let mut chain: Option<LirStmt> = None;

        for case in cases.iter().rev() {
            let (cond_opt, case_stmts, case_ret) =
                self.lower_match_case(&target_atom, &case.pattern, &case.body, ret_type)?;

            let else_body = chain.take().map_or_else(Vec::new, |next| vec![next]);

            // Last remaining arm: treat as exhaustive fallback (cond=true)
            let cond = if else_body.is_empty() {
                LirAtom::Bool(true)
            } else {
                cond_opt.unwrap_or(LirAtom::Bool(true))
            };

            // Determine if this case genuinely returns a value:
            // - case_ret: explicit return value (bare expression or return stmt)
            // - fallback: terminal IfReturn/TryCatch from nested control flow
            // If neither, this is a side-effect-only case → then_ret = None
            let genuine_ret = case_ret
                .or_else(|| fallback_return_atom_from_terminal_stmt(&case_stmts));

            let then_ret = genuine_ret.map(|ret| {
                // If the case diverges (e.g. raise), ret may be Unit-typed
                // even though the function returns non-Unit. Use a placeholder
                // of the correct type so codegen can register a proper WASM local.
                if matches!(ret.typ(), Type::Unit) && !matches!(ret_type, Type::Unit) {
                    default_atom_for_type(ret_type)
                } else {
                    ret
                }
            });

            chain = Some(LirStmt::IfReturn {
                cond,
                then_body: case_stmts,
                then_ret,
                else_body,
                else_ret: None,
                ret_type: ret_type.clone(),
            });
        }

        if let Some(stmt) = chain {
            self.stmts.push(stmt);
        }
        Ok(())
    }

    /// Lower a while loop into a LirStmt::Loop
    fn lower_while_stmt(
        &mut self,
        cond: &MirExpr,
        body: &[MirStmt],
        ret_type: &Type,
    ) -> Result<(), LirLowerError> {
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = self.vars.clone();
        let saved_sem_vars = self.semantic_vars.clone();

        // Lower condition (produces temp stmts + a condition atom)
        let cond_atom = self.lower_expr_to_atom(cond)?;
        // Negate: break when condition is false
        let neg_cond = self.bind_expr_to_temp(
            LirExpr::Binary {
                op: BinaryOp::Eq,
                lhs: cond_atom,
                rhs: LirAtom::Bool(false),
                typ: Type::Bool,
            },
            Type::Bool,
        );
        let cond_stmts = std::mem::take(&mut self.stmts);

        // Lower body
        for stmt in body {
            self.lower_stmt(stmt, ret_type)?;
        }
        let body_stmts = std::mem::replace(&mut self.stmts, saved_stmts);

        self.vars = saved_vars;
        self.semantic_vars = saved_sem_vars;

        self.stmts.push(LirStmt::Loop {
            cond_stmts,
            cond: neg_cond,
            body: body_stmts,
        });
        Ok(())
    }

    /// Lower a match expression to an atom (for use in expression position).
    /// Creates a temp result variable and assigns to it in each branch.
    fn lower_match_expr(
        &mut self,
        target: &MirExpr,
        cases: &[MirMatchCase],
    ) -> Result<LirAtom, LirLowerError> {
        let target_atom = self.lower_expr_to_atom(target)?;

        if cases.is_empty() {
            return Ok(LirAtom::Unit);
        }

        // Lower all cases first to get accurate types from actual lowered values
        let mut lowered_cases: Vec<(Option<LirAtom>, Vec<LirStmt>, LirAtom)> = Vec::new();
        for case in cases.iter().rev() {
            let result =
                self.lower_match_case_expr(&target_atom, &case.pattern, &case.body)?;
            lowered_cases.push(result);
        }

        // Infer semantic result type from the first non-Unit case value
        let semantic_result_type = lowered_cases
            .iter()
            .find_map(|(_, _, val)| {
                let t = val.typ();
                if matches!(t, Type::Unit) {
                    None
                } else {
                    // For Var atoms, look up their semantic type
                    match val {
                        LirAtom::Var { name, .. } => {
                            self.semantic_vars.get(name).cloned().or(Some(t))
                        }
                        _ => Some(t),
                    }
                }
            })
            .unwrap_or(Type::Unit);
        let result_type = wasm_type(&semantic_result_type);

        let result_name = self.new_temp();
        // Pre-register the result variable with correct semantic type
        self.vars.insert(result_name.clone(), result_type.clone());
        self.semantic_vars
            .insert(result_name.clone(), semantic_result_type);
        // Initialize with a placeholder
        self.stmts.push(LirStmt::Let {
            name: result_name.clone(),
            typ: result_type.clone(),
            expr: LirExpr::Atom(default_atom_for_type(&result_type)),
        });

        // Build nested If chain that assigns to result_name
        let mut chain: Option<LirStmt> = None;

        for (cond_opt, mut case_stmts, case_val) in lowered_cases {
            let else_body = chain.take().map_or_else(Vec::new, |next| vec![next]);

            let cond = if else_body.is_empty() {
                LirAtom::Bool(true) // last arm = exhaustive fallback
            } else {
                cond_opt.unwrap_or(LirAtom::Bool(true))
            };

            // Assign the case value to the result variable
            case_stmts.push(LirStmt::Let {
                name: result_name.clone(),
                typ: result_type.clone(),
                expr: LirExpr::Atom(case_val),
            });

            chain = Some(LirStmt::If {
                cond,
                then_body: case_stmts,
                else_body,
            });
        }

        if let Some(stmt) = chain {
            self.stmts.push(stmt);
        }

        Ok(LirAtom::Var {
            name: result_name,
            typ: result_type,
        })
    }

    /// Lower a single match case for expression position.
    /// Returns (condition, body_stmts, value_atom) — the value to assign.
    fn lower_match_case_expr(
        &mut self,
        target: &LirAtom,
        pattern: &MirPattern,
        body: &[MirStmt],
    ) -> Result<(Option<LirAtom>, Vec<LirStmt>, LirAtom), LirLowerError> {
        let mut conds = Vec::new();
        let mut bindings: HashMap<Symbol, LirAtom> = HashMap::new();

        let stmts_before = self.stmts.len();
        self.collect_pattern_conditions_and_bindings(target, pattern, &mut conds, &mut bindings)?;

        // Guard field extraction stmts for Constructor patterns (same as lower_match_case)
        let has_ctor_fields = matches!(pattern, MirPattern::Constructor { fields, .. } if !fields.is_empty());
        let guarded_stmts: Vec<LirStmt> = if has_ctor_fields {
            let tag_check_end = stmts_before + 2;
            if self.stmts.len() > tag_check_end {
                self.stmts.drain(tag_check_end..).collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        let combined_cond = if conds.is_empty() {
            None
        } else if !guarded_stmts.is_empty() && !conds.is_empty() {
            Some(conds[0].clone())
        } else {
            Some(self.combine_bool_conditions(&conds))
        };

        // Create scope with bindings
        let mut case_vars = self.vars.clone();
        let mut case_sem_vars = self.semantic_vars.clone();
        let semantic_bindings = self.collect_pattern_semantic_types(target, pattern);
        for (name, atom) in &bindings {
            case_vars.insert(name.clone(), atom.typ());
            let sem_type = semantic_bindings
                .get(name)
                .cloned()
                .unwrap_or_else(|| atom.typ());
            case_sem_vars.insert(name.clone(), sem_type);
        }

        // Lower the body: all but the last statement, then the last produces the value
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = std::mem::replace(&mut self.vars, case_vars);
        let saved_semantic_vars = std::mem::replace(&mut self.semantic_vars, case_sem_vars);

        // Prepend guarded pattern stmts
        self.stmts.extend(guarded_stmts);

        // Prepend binding let-statements
        for (name, atom) in &bindings {
            let typ = atom.typ();
            self.stmts.push(LirStmt::Let {
                name: name.clone(),
                typ,
                expr: LirExpr::Atom(atom.clone()),
            });
        }

        let value_atom = if body.is_empty() {
            LirAtom::Unit
        } else {
            // Process all but the last statement
            let last_idx = body.len() - 1;
            for stmt in &body[..last_idx] {
                self.lower_stmt(stmt, &Type::Unit)?;
            }
            // The last statement produces the value
            match &body[last_idx] {
                MirStmt::Expr(expr) => self.lower_expr_to_atom(expr)?,
                MirStmt::Return(expr) => {
                    // Return diverges from the function — still produce the value
                    // (the IfReturn will handle the actual return)
                    self.lower_expr_to_atom(expr)?
                }
                _ => {
                    self.lower_stmt(&body[last_idx], &Type::Unit)?;
                    LirAtom::Unit
                }
            }
        };

        let case_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
        self.vars = saved_vars;
        self.semantic_vars = saved_semantic_vars;

        Ok((combined_cond, case_stmts, value_atom))
    }

    /// Lower a single match case, returning (condition, body_stmts, return_atom)
    fn lower_match_case(
        &mut self,
        target: &LirAtom,
        pattern: &MirPattern,
        body: &[MirStmt],
        ret_type: &Type,
    ) -> Result<(Option<LirAtom>, Vec<LirStmt>, Option<LirAtom>), LirLowerError> {
        let mut conds = Vec::new();
        let mut bindings: HashMap<Symbol, LirAtom> = HashMap::new();

        // Track stmts emitted by pattern extraction
        let stmts_before = self.stmts.len();
        self.collect_pattern_conditions_and_bindings(target, pattern, &mut conds, &mut bindings)?;

        // For Constructor patterns with fields, the first 2 stmts are the
        // top-level tag check (ObjectTag read + Eq), which only reads offset 0
        // and is always safe. Remaining stmts (field extractions, nested tag
        // checks) dereference field pointers and are only valid when the tag
        // matches. Move them inside the case body so they execute only after
        // the tag check passes.
        let has_ctor_fields = matches!(pattern, MirPattern::Constructor { fields, .. } if !fields.is_empty());
        let guarded_stmts: Vec<LirStmt> = if has_ctor_fields {
            let tag_check_end = stmts_before + 2;
            if self.stmts.len() > tag_check_end {
                self.stmts.drain(tag_check_end..).collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        // For Constructor patterns with guarded stmts, use only the outer
        // tag check as the condition (first cond). Inner conditions from
        // nested patterns are inside guarded_stmts and will be validated
        // inside the then_body.
        let combined_cond = if conds.is_empty() {
            None
        } else if !guarded_stmts.is_empty() && !conds.is_empty() {
            Some(conds[0].clone())
        } else {
            Some(self.combine_bool_conditions(&conds))
        };

        // Create scope with bindings
        let mut case_vars = self.vars.clone();
        let mut case_sem_vars = self.semantic_vars.clone();
        // Compute semantic types for pattern-bound variables
        let semantic_bindings = self.collect_pattern_semantic_types(target, pattern);
        for (name, atom) in &bindings {
            case_vars.insert(name.clone(), atom.typ());
            let sem_type = semantic_bindings
                .get(name)
                .cloned()
                .unwrap_or_else(|| atom.typ());
            case_sem_vars.insert(name.clone(), sem_type);
        }

        // Lower body, capturing the last expression's value as case_ret.
        // Save state and set up scoped vars.
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = std::mem::replace(&mut self.vars, case_vars);
        let saved_semantic_vars = std::mem::replace(&mut self.semantic_vars, case_sem_vars);

        // Prepend guarded pattern stmts (field extractions, nested checks)
        // before the binding lets — they define the atoms used by bindings.
        self.stmts.extend(guarded_stmts);

        // Prepend binding let-statements
        for (name, atom) in bindings {
            let typ = atom.typ();
            self.stmts.push(LirStmt::Let {
                name,
                typ: typ.clone(),
                expr: LirExpr::Atom(atom),
            });
        }

        let case_ret = if body.is_empty() {
            None
        } else {
            // Process all but the last statement
            let last_idx = body.len() - 1;
            for stmt in &body[..last_idx] {
                self.lower_stmt(stmt, ret_type)?;
            }
            // The last statement: capture its value if it's an expression
            match &body[last_idx] {
                MirStmt::Expr(expr) => match expr {
                    MirExpr::If {
                        cond,
                        then_body,
                        else_body,
                    } => {
                        self.lower_if_stmt(cond, then_body, else_body.as_deref(), ret_type)?;
                        None
                    }
                    MirExpr::Match { target, cases } => {
                        self.lower_match_stmt(target, cases, ret_type)?;
                        None
                    }
                    MirExpr::While { cond, body: wb } => {
                        self.lower_while_stmt(cond, wb, ret_type)?;
                        None
                    }
                    _ => {
                        let atom = self.lower_expr_to_atom(expr)?;
                        if matches!(atom.typ(), Type::Unit) {
                            None
                        } else {
                            Some(atom)
                        }
                    }
                },
                MirStmt::Return(expr) => {
                    let atom = self.lower_expr_to_atom(expr)?;
                    if matches!(atom.typ(), Type::Unit) {
                        None
                    } else {
                        Some(atom)
                    }
                }
                _ => {
                    self.lower_stmt(&body[last_idx], ret_type)?;
                    None
                }
            }
        };

        let case_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
        self.vars = saved_vars;
        self.semantic_vars = saved_semantic_vars;

        Ok((combined_cond, case_stmts, case_ret))
    }

    /// Collect pattern matching conditions and variable bindings
    fn collect_pattern_conditions_and_bindings(
        &mut self,
        target: &LirAtom,
        pattern: &MirPattern,
        conds: &mut Vec<LirAtom>,
        bindings: &mut HashMap<Symbol, LirAtom>,
    ) -> Result<(), LirLowerError> {
        match pattern {
            MirPattern::Wildcard => {} // always matches
            MirPattern::Variable(name, _sigil) => {
                bindings.insert(*name, target.clone());
            }
            MirPattern::Literal(lit) => {
                let lit_atom = literal_to_atom(lit);
                let cond = self.bind_expr_to_temp(
                    LirExpr::Binary {
                        op: BinaryOp::Eq,
                        lhs: target.clone(),
                        rhs: lit_atom,
                        typ: Type::Bool,
                    },
                    Type::Bool,
                );
                conds.push(cond);
            }
            MirPattern::Constructor { name, fields } => {
                // Use enum definition arity for tag (not pattern field count)
                let def_arity = lookup_constructor_field_labels(name.as_str(), self.enum_defs)
                    .map(|labels| labels.len())
                    .unwrap_or(fields.len());

                // Tag check
                let tag_atom = self.bind_expr_to_temp(
                    LirExpr::ObjectTag {
                        value: target.clone(),
                        typ: Type::I64,
                    },
                    Type::I64,
                );
                let expected_tag = constructor_tag(name.as_str(), def_arity);
                let tag_cond = self.bind_expr_to_temp(
                    LirExpr::Binary {
                        op: BinaryOp::Eq,
                        lhs: tag_atom,
                        rhs: LirAtom::Int(expected_tag),
                        typ: Type::Bool,
                    },
                    Type::Bool,
                );
                conds.push(tag_cond);

                // Resolve field types from enum definition so unpack
                // conversion (I64 → I32 for Bool, etc.) is correct.
                let target_sem_type = match target {
                    LirAtom::Var { name: vn, .. } => self
                        .semantic_vars
                        .get(vn)
                        .cloned()
                        .unwrap_or_else(|| target.typ()),
                    _ => target.typ(),
                };
                let resolved_field_types =
                    resolve_constructor_field_types(name.as_str(), &target_sem_type, self.enum_defs);

                // Compute sorted field order for labeled constructors
                let sorted_indices = constructor_sorted_field_indices(name.as_str(), self.enum_defs);

                // Field checks — use sorted index for memory layout
                for (pat_idx, (label, field_pat)) in fields.iter().enumerate() {
                    // Determine the memory index for this field:
                    // - If the pattern has a label, find its sorted position
                    // - Otherwise, use the pattern index mapped through sorted order
                    let mem_idx = if let Some(ref si) = sorted_indices {
                        if let Some(lbl) = label {
                            // Find this label's definition index, then map to sorted index
                            lookup_constructor_field_labels(name.as_str(), self.enum_defs)
                                .and_then(|labels| {
                                    labels.iter().position(|l| l.as_ref() == Some(lbl))
                                })
                                .map(|def_idx| si[def_idx])
                                .unwrap_or(pat_idx)
                        } else {
                            // Positional: pat_idx is definition order
                            si.get(pat_idx).copied().unwrap_or(pat_idx)
                        }
                    } else {
                        pat_idx
                    };

                    // Resolve semantic field type using definition index (not memory index)
                    let def_idx = if let Some(lbl) = label {
                        lookup_constructor_field_labels(name.as_str(), self.enum_defs)
                            .and_then(|labels| {
                                labels.iter().position(|l| l.as_ref() == Some(lbl))
                            })
                            .unwrap_or(pat_idx)
                    } else {
                        pat_idx
                    };
                    let semantic_ft = resolved_field_types
                        .as_ref()
                        .and_then(|fts| fts.get(def_idx))
                        .cloned();
                    let wasm_ft = semantic_ft
                        .as_ref()
                        .map(|ft| wasm_type(ft))
                        .unwrap_or_else(|| {
                            // Generic/polymorphic constructor fields — I64 is the correct
                            // WASM representation for unresolved type parameters
                            tracing::debug!(
                                "constructor '{}' field at index {}: unresolved type, using I64",
                                name, def_idx
                            );
                            Type::I64
                        });
                    let field_atom = self.bind_expr_to_temp(
                        LirExpr::ObjectField {
                            value: target.clone(),
                            index: mem_idx,
                            typ: wasm_ft.clone(),
                        },
                        wasm_ft,
                    );
                    // Register semantic type for nested pattern matching
                    if let (LirAtom::Var { name: fn_name, .. }, Some(sft)) =
                        (&field_atom, &semantic_ft)
                    {
                        self.semantic_vars.insert(fn_name.clone(), sft.clone());
                    }
                    self.collect_pattern_conditions_and_bindings(
                        &field_atom,
                        field_pat,
                        conds,
                        bindings,
                    )?;
                }
            }
            MirPattern::Record(fields, _open) => {
                // Resolve field types from the target's semantic type
                let semantic_type = match target {
                    LirAtom::Var { name, .. } => self
                        .semantic_vars
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| target.typ()),
                    _ => target.typ(),
                };
                let record_field_types: Vec<(String, Type)> =
                    if let Type::Record(rt_fields) = strip_linear(&semantic_type) {
                        rt_fields.clone()
                    } else {
                        Vec::new()
                    };
                for (name, field_pat) in fields.iter() {
                    // Find the sorted index and type from the record layout
                    let (sorted_idx, field_type) = record_field_types
                        .iter()
                        .enumerate()
                        .find(|(_, (n, _))| n == name)
                        .map(|(i, (_, t))| (i, t.clone()))
                        .ok_or_else(|| LirLowerError::UnresolvedType {
                            detail: format!("record field '{}' not found in record type {:?}", name, semantic_type),
                            span: 0..0,
                        })?;
                    let field_atom = self.bind_expr_to_temp(
                        LirExpr::ObjectField {
                            value: target.clone(),
                            index: sorted_idx,
                            typ: field_type.clone(),
                        },
                        field_type,
                    );
                    self.collect_pattern_conditions_and_bindings(
                        &field_atom,
                        field_pat,
                        conds,
                        bindings,
                    )?;
                }
            }
        }
        Ok(())
    }

    /// Collect semantic types for pattern-bound variables by walking the pattern
    /// and looking up target's semantic type.
    fn collect_pattern_semantic_types(
        &self,
        target: &LirAtom,
        pattern: &MirPattern,
    ) -> HashMap<Symbol, Type> {
        let mut result = HashMap::new();
        let target_sem_type = match target {
            LirAtom::Var { name, .. } => self
                .semantic_vars
                .get(name)
                .cloned()
                .unwrap_or_else(|| target.typ()),
            _ => target.typ(),
        };
        self.walk_pattern_semantic_types(&target_sem_type, pattern, &mut result);
        result
    }

    fn walk_pattern_semantic_types(
        &self,
        sem_type: &Type,
        pattern: &MirPattern,
        out: &mut HashMap<Symbol, Type>,
    ) {
        match pattern {
            MirPattern::Variable(name, _) => {
                out.insert(*name, sem_type.clone());
            }
            MirPattern::Record(fields, _) => {
                let record_fields: Vec<(String, Type)> =
                    if let Type::Record(rf) = strip_linear(sem_type) {
                        rf.clone()
                    } else {
                        Vec::new()
                    };
                for (name, field_pat) in fields {
                    let field_type = record_fields
                        .iter()
                        .find(|(n, _)| *n == name.as_str())
                        .map(|(_, t)| t.clone())
                        .unwrap_or_else(|| {
                            tracing::debug!(
                                "record field '{}' not found in semantic type {:?}, using I64",
                                name, sem_type
                            );
                            Type::I64
                        });
                    self.walk_pattern_semantic_types(&field_type, field_pat, out);
                }
            }
            MirPattern::Constructor { name, fields } => {
                // Look up constructor's enum definition to resolve field types
                let field_types = resolve_constructor_field_types(name.as_str(), sem_type, self.enum_defs);
                for (pat_idx, (label, field_pat)) in fields.iter().enumerate() {
                    // Resolve definition index for this field (label-aware)
                    let def_idx = if let Some(lbl) = label {
                        lookup_constructor_field_labels(name.as_str(), self.enum_defs)
                            .and_then(|labels| {
                                labels.iter().position(|l| l.as_ref() == Some(lbl))
                            })
                            .unwrap_or(pat_idx)
                    } else {
                        pat_idx
                    };
                    let ft = field_types
                        .as_ref()
                        .and_then(|fts| fts.get(def_idx))
                        .cloned()
                        .unwrap_or_else(|| {
                            // Generic/polymorphic constructor — I64 is correct for unresolved type params
                            tracing::debug!(
                                "constructor '{}' field at index {}: unresolved type, using I64",
                                name, def_idx
                            );
                            Type::I64
                        });
                    self.walk_pattern_semantic_types(&ft, field_pat, out);
                }
            }
            MirPattern::Wildcard | MirPattern::Literal(_) => {}
        }
    }

    /// Combine bool conditions with And operations
    fn combine_bool_conditions(&mut self, conds: &[LirAtom]) -> LirAtom {
        if conds.len() == 1 {
            return conds[0].clone();
        }
        let mut result = conds[0].clone();
        for cond in &conds[1..] {
            result = self.bind_expr_to_temp(
                LirExpr::Binary {
                    op: BinaryOp::And,
                    lhs: result,
                    rhs: cond.clone(),
                    typ: Type::Bool,
                },
                Type::Bool,
            );
        }
        result
    }

    /// Lower a MIR expression to an atom (flattening complex sub-expressions)
    fn lower_expr_to_atom(&mut self, expr: &MirExpr) -> Result<LirAtom, LirLowerError> {
        match expr {
            MirExpr::Literal(lit) => Ok(literal_to_atom(lit)),
            MirExpr::Variable(name) => {
                // Use semantic type (mapped through wasm_type) when available,
                // so that e.g. String stays String rather than I64 from ObjectField
                let typ = self
                    .semantic_vars
                    .get(name)
                    .map(|st| wasm_type(st))
                    .or_else(|| self.vars.get(name).cloned())
                    .ok_or_else(|| LirLowerError::UnresolvedType {
                        detail: format!("variable '{}' not in scope", name),
                        span: 0..0,
                    })?;
                Ok(LirAtom::Var {
                    name: name.clone(),
                    typ,
                })
            }
            MirExpr::BinaryOp(lhs, op, rhs) => {
                let lhs_atom = self.lower_expr_to_atom(lhs)?;
                let rhs_atom = self.lower_expr_to_atom(rhs)?;
                let result_type = infer_binary_type(*op, &lhs_atom.typ(), &rhs_atom.typ());
                Ok(self.bind_expr_to_temp(
                    LirExpr::Binary {
                        op: *op,
                        lhs: lhs_atom,
                        rhs: rhs_atom,
                        typ: result_type.clone(),
                    },
                    result_type,
                ))
            }
            MirExpr::Call {
                func,
                args,
                ret_type,
            } => {
                // Evaluate args left-to-right, then sort by label
                let mut lir_args: Vec<(Symbol, LirAtom)> = Vec::new();
                for (label, expr) in args {
                    let atom = self.lower_expr_to_atom(expr)?;
                    lir_args.push((*label, atom));
                }
                lir_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                let typ = wasm_type(ret_type);
                Ok(self.bind_expr_to_temp_semantic(
                    LirExpr::Call {
                        func: func.clone(),
                        args: lir_args,
                        typ: typ.clone(),
                    },
                    typ,
                    ret_type.clone(),
                ))
            }
            MirExpr::Constructor { name, args } => {
                // 1. Evaluate args left-to-right (preserving side-effect order)
                let mut labeled_args: Vec<(Option<Symbol>, LirAtom)> = Vec::new();
                for (label, arg) in args {
                    let atom = self.lower_expr_to_atom(arg)?;
                    labeled_args.push((*label, atom));
                }

                // 2. Fill in missing labels from enum definition (positional → labeled)
                let def_labels = lookup_constructor_field_labels(name.as_str(), self.enum_defs);
                if let Some(ref labels) = def_labels {
                    for (i, (label, _)) in labeled_args.iter_mut().enumerate() {
                        if label.is_none() {
                            if let Some(def_label) = labels.get(i) {
                                if let Some(l) = def_label {
                                    *label = Some(*l);
                                }
                            }
                        }
                    }
                }

                // 3. Sort labeled args by label (lexicographic), unlabeled stay in order
                let all_labeled = labeled_args.iter().all(|(l, _)| l.is_some());
                if all_labeled && labeled_args.len() > 1 {
                    labeled_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                }

                // 4. Strip labels
                let lir_args: Vec<LirAtom> = labeled_args.into_iter().map(|(_, atom)| atom).collect();
                let typ = Type::I64; // object pointer
                Ok(self.bind_expr_to_temp(
                    LirExpr::Constructor {
                        name: name.clone(),
                        args: lir_args,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Record(fields) => {
                let mut lir_fields = Vec::new();
                for (name, expr) in fields {
                    let atom = self.lower_expr_to_atom(expr)?;
                    lir_fields.push((name.clone(), atom));
                }
                // Sort fields by name for consistent layout
                lir_fields.sort_by(|(a, _), (b, _)| a.cmp(b));
                let typ = Type::I64; // object pointer
                Ok(self.bind_expr_to_temp(
                    LirExpr::Record {
                        fields: lir_fields,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Array(items) => {
                // Arrays are encoded as records with numeric indices
                let mut lir_items = Vec::new();
                for (idx, item) in items.iter().enumerate() {
                    let atom = self.lower_expr_to_atom(item)?;
                    lir_items.push((Symbol::from(idx.to_string()), atom));
                }
                let typ = Type::I64;
                Ok(self.bind_expr_to_temp(
                    LirExpr::Record {
                        fields: lir_items,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Index(arr, idx) => {
                let arr_atom = self.lower_expr_to_atom(arr)?;
                let idx_atom = self.lower_expr_to_atom(idx)?;
                // Index is compiled as object field access with dynamic index
                // For now, emit as a call to an intrinsic
                let typ = Type::I64;
                Ok(self.bind_expr_to_temp(
                    LirExpr::Call {
                        func: Symbol::from("__array_get"),
                        args: vec![(Symbol::from("arr"), arr_atom), (Symbol::from("idx"), idx_atom)],
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::FieldAccess(expr, field) => {
                // Resolve the receiver's semantic type to determine field index and type
                let receiver_semantic_type = self.infer_semantic_type(expr);
                let obj_atom = self.lower_expr_to_atom(expr)?;

                let (idx, field_type) = resolve_field_access(&receiver_semantic_type, field.as_str())?;

                let typ = wasm_type(&field_type);
                Ok(self.bind_expr_to_temp(
                    LirExpr::ObjectField {
                        value: obj_atom,
                        index: idx,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::If {
                cond,
                then_body,
                else_body,
            } => self.lower_if_expr(cond, then_body, else_body.as_deref()),
            MirExpr::Match { target, cases } => self.lower_match_expr(target, cases),
            MirExpr::While { .. } => Err(LirLowerError::UnsupportedExpression {
                detail: "While loop in atom position; should be lowered at statement level"
                    .to_string(),
                span: 0..0,
            }),
            MirExpr::Borrow(name) => {
                let typ = self.vars.get(name).cloned()
                    .ok_or_else(|| LirLowerError::UnresolvedType {
                        detail: format!("borrowed variable '{}' not in scope", name),
                        span: 0..0,
                    })?;
                Ok(LirAtom::Var {
                    name: name.clone(),
                    typ,
                })
            }
            MirExpr::Raise(expr) => {
                let atom = self.lower_expr_to_atom(expr)?;
                let typ = Type::Unit; // raise doesn't return
                Ok(self.bind_expr_to_temp(
                    LirExpr::Raise {
                        value: atom,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::FuncRef(name) => {
                let typ = Type::I64; // funcref stored as i64 closure pointer
                Ok(self.bind_expr_to_temp(
                    LirExpr::FuncRef {
                        func: *name,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Closure { func, captures } => {
                // Lower each captured variable to an atom
                let capture_atoms: Vec<(Symbol, LirAtom)> = captures
                    .iter()
                    .map(|name| {
                        let typ = self
                            .semantic_vars
                            .get(name)
                            .map(|st| wasm_type(st))
                            .or_else(|| self.vars.get(name).cloned())
                            .unwrap_or(Type::I64);
                        (*name, LirAtom::Var { name: *name, typ })
                    })
                    .collect();
                let typ = Type::I64; // closure pointer as i64
                Ok(self.bind_expr_to_temp(
                    LirExpr::Closure {
                        func: *func,
                        captures: capture_atoms,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::CallIndirect {
                callee,
                args,
                ret_type: _,
                callee_type: _,
            } => {
                let callee_atom = self.lower_expr_to_atom(callee)?;
                // Resolve the actual Arrow type from semantic_vars
                let callee_type = if let MirExpr::Variable(name) = callee.as_ref() {
                    self.semantic_vars.get(name).cloned().unwrap_or(Type::I64)
                } else {
                    Type::I64
                };
                // Infer return type from the Arrow type
                let ret_type = if let Type::Arrow(_, ret, _, _) = &callee_type {
                    *ret.clone()
                } else {
                    Type::I64
                };
                let mut lir_args: Vec<(Symbol, LirAtom)> = Vec::new();
                for (label, expr) in args {
                    let atom = self.lower_expr_to_atom(expr)?;
                    lir_args.push((*label, atom));
                }
                lir_args.sort_by(|(a, _), (b, _)| a.cmp(b));
                let typ = wasm_type(&ret_type);
                Ok(self.bind_expr_to_temp_semantic(
                    LirExpr::CallIndirect {
                        callee: callee_atom,
                        args: lir_args,
                        typ: typ.clone(),
                        callee_type,
                    },
                    typ,
                    ret_type,
                ))
            }
        }
    }

    /// Infer the semantic (pre-wasm) type of a MIR expression by looking up
    /// variable bindings in semantic_vars.
    fn infer_semantic_type(&self, expr: &MirExpr) -> Type {
        match expr {
            MirExpr::Variable(name) => self.semantic_vars.get(name).cloned()
                .unwrap_or_else(|| {
                    tracing::debug!("variable '{}' not in semantic_vars during type inference, using I64", name);
                    Type::I64
                }),
            MirExpr::Call { ret_type, .. } => ret_type.clone(),
            MirExpr::Record(fields) => {
                let mut field_types: Vec<(String, Type)> = fields
                    .iter()
                    .map(|(name, expr)| (name.to_string(), self.infer_semantic_type(expr)))
                    .collect();
                field_types.sort_by(|a, b| a.0.cmp(&b.0));
                Type::Record(field_types)
            }
            MirExpr::Constructor { .. } => Type::I64,
            MirExpr::Literal(lit) => match lit {
                Literal::Int(_) => Type::I64,
                Literal::Float(_) => Type::F64,
                Literal::Bool(_) => Type::Bool,
                Literal::Char(_) => Type::Char,
                Literal::String(_) => Type::String,
                Literal::Unit => Type::Unit,
            },
            MirExpr::FieldAccess(receiver, field) => {
                let receiver_type = self.infer_semantic_type(receiver);
                resolve_field_access(&receiver_type, field.as_str())
                    .map(|(_, typ)| typ)
                    .unwrap_or_else(|e| {
                        tracing::debug!("field '{}' not resolvable on type {:?}: {}", field, receiver_type, e);
                        Type::I64
                    })
            }
            MirExpr::If { then_body, .. } => {
                // Infer from last statement of then branch
                if let Some(last) = then_body.last() {
                    self.infer_stmt_type(last)
                } else {
                    Type::Unit
                }
            }
            MirExpr::Match { cases, .. } => {
                // Infer from first case body
                if let Some(case) = cases.first() {
                    if let Some(last) = case.body.last() {
                        self.infer_stmt_type(last)
                    } else {
                        Type::Unit
                    }
                } else {
                    Type::Unit
                }
            }
            MirExpr::Array(items) => {
                let elem_type = items
                    .first()
                    .map(|e| self.infer_semantic_type(e))
                    .unwrap_or_else(|| {
                        tracing::debug!("empty array literal, using I64 element type");
                        Type::I64
                    });
                Type::Array(Box::new(elem_type))
            }
            MirExpr::Index(arr, _) => {
                let arr_type = self.infer_semantic_type(arr);
                match arr_type {
                    Type::Array(elem) => *elem,
                    other => {
                        tracing::debug!("index operation on non-array type {:?}, using I64", other);
                        Type::I64
                    }
                }
            }
            MirExpr::BinaryOp(lhs, op, _) => {
                if op.is_comparison() || matches!(op, BinaryOp::And | BinaryOp::Or) {
                    Type::Bool
                } else {
                    self.infer_semantic_type(lhs)
                }
            }
            MirExpr::While { .. } => Type::Unit,
            MirExpr::Borrow(name) => self.semantic_vars.get(name).cloned()
                .unwrap_or_else(|| {
                    tracing::debug!("borrowed variable '{}' not in semantic_vars during type inference, using I64", name);
                    Type::I64
                }),
            // Raise never returns; I64 is a placeholder (no Type::Never exists)
            MirExpr::Raise(_) => Type::I64,
            MirExpr::FuncRef(_) | MirExpr::Closure { .. } => Type::I64,
            MirExpr::CallIndirect { ret_type, .. } => ret_type.clone(),
        }
    }

    fn infer_stmt_type(&self, stmt: &MirStmt) -> Type {
        match stmt {
            MirStmt::Expr(e) | MirStmt::Return(e) => self.infer_semantic_type(e),
            MirStmt::Let { typ, .. } => typ.clone(),
            _ => Type::Unit,
        }
    }

    /// Lower a block of statements, returning (statements, optional return atom)
    fn lower_block(
        &mut self,
        stmts: &[MirStmt],
        ret_type: &Type,
    ) -> Result<(Vec<LirStmt>, Option<LirAtom>), LirLowerError> {
        let vars = self.vars.clone();
        let sem_vars = self.semantic_vars.clone();
        self.lower_block_with_vars(stmts, ret_type, vars, sem_vars)
    }

    fn lower_block_with_vars(
        &mut self,
        stmts: &[MirStmt],
        ret_type: &Type,
        vars: HashMap<Symbol, Type>,
        semantic_vars: HashMap<Symbol, Type>,
    ) -> Result<(Vec<LirStmt>, Option<LirAtom>), LirLowerError> {
        // Save state
        let saved_stmts = std::mem::take(&mut self.stmts);
        let saved_vars = std::mem::replace(&mut self.vars, vars);
        let saved_semantic_vars = std::mem::replace(&mut self.semantic_vars, semantic_vars);

        let mut ret_atom = None;
        for stmt in stmts {
            if let Some(atom) = self.lower_stmt(stmt, ret_type)? {
                ret_atom = Some(atom);
                break;
            }
        }

        let block_stmts = std::mem::replace(&mut self.stmts, saved_stmts);
        self.vars = saved_vars;
        self.semantic_vars = saved_semantic_vars;

        Ok((block_stmts, ret_atom))
    }
}

/// Convert a literal to an atom
fn literal_to_atom(lit: &Literal) -> LirAtom {
    match lit {
        Literal::Int(i) => LirAtom::Int(*i),
        Literal::Float(f) => LirAtom::Float(*f),
        Literal::Bool(b) => LirAtom::Bool(*b),
        Literal::Char(c) => LirAtom::Char(*c),
        Literal::String(s) => LirAtom::String(s.clone()),
        Literal::Unit => LirAtom::Unit,
    }
}

/// Resolve a field access on a semantic type.
/// Returns (field_index, field_type).
/// Fields are sorted alphabetically (matching record layout in codegen).
fn resolve_field_access(
    receiver_type: &Type,
    field_name: &str,
) -> Result<(usize, Type), LirLowerError> {
    match receiver_type {
        Type::Record(fields) => {
            let mut sorted: Vec<(String, Type)> = fields.clone();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (idx, (name, typ)) in sorted.iter().enumerate() {
                if name == field_name {
                    return Ok((idx, typ.clone()));
                }
            }
            Err(LirLowerError::UnsupportedExpression {
                detail: format!(
                    "field '{}' not found in record type {:?}",
                    field_name, receiver_type
                ),
                span: 0..0,
            })
        }
        _ => Err(LirLowerError::UnsupportedExpression {
            detail: format!(
                "field access '.{}' on non-record type {:?}",
                field_name, receiver_type
            ),
            span: 0..0,
        }),
    }
}

/// Map a high-level AST type to its WASM-level representation.
/// Primitives pass through with semantic type preserved (Bool stays Bool, etc.).
/// Heap-allocated and compound types collapse to I64 (object pointer).
fn wasm_type(typ: &Type) -> Type {
    match typ {
        // Primitives: keep semantic type for downstream codegen
        Type::I32 | Type::I64 | Type::F32 | Type::F64 | Type::Bool | Type::Char | Type::String | Type::Unit => {
            typ.clone()
        }
        // Literal types: resolve to concrete numeric type
        Type::IntLit => Type::I64,
        Type::FloatLit => Type::F64,
        // Rows are not runtime values
        Type::Row(_, _) => Type::Unit,
        // All compound/heap types: classify via wasm_repr
        _ => match typ.wasm_repr() {
            WasmRepr::I64 => Type::I64,
            WasmRepr::I32 | WasmRepr::F32 | WasmRepr::F64 | WasmRepr::Unit => {
                unreachable!("compound type {typ} has unexpected wasm_repr")
            }
        },
    }
}

/// Infer the result type of a binary operation (matches old lower.rs logic exactly)
fn infer_binary_type(op: BinaryOp, lhs: &Type, rhs: &Type) -> Type {
    if op.is_comparison() {
        return Type::Bool;
    }
    if op == BinaryOp::And || op == BinaryOp::Or {
        return Type::Bool;
    }
    if op == BinaryOp::Concat
        || (op == BinaryOp::Add && matches!(lhs, Type::String) && matches!(rhs, Type::String))
    {
        return Type::String;
    }
    if op.is_float_op()
        || matches!(lhs, Type::F32 | Type::F64)
        || matches!(rhs, Type::F32 | Type::F64)
    {
        if matches!(lhs, Type::F32) || matches!(rhs, Type::F32) {
            Type::F32
        } else {
            Type::F64
        }
    } else if matches!(lhs, Type::I32) || matches!(rhs, Type::I32) {
        Type::I32
    } else {
        Type::I64
    }
}

/// Check if the terminal statement in a block is a genuine return-containing
/// statement (IfReturn or TryCatch with returns). Unlike `fallback_return_atom_from_terminal_stmt`,
/// this does NOT match trailing Let bindings, which are not actual returns.
/// Try to extract a return atom from the last statement in a block.
/// Only matches genuine return-producing statements (IfReturn, TryCatch).
/// Does NOT match Let bindings — a trailing `let x = expr` is a side effect,
/// not a return value. Treating Let as a return was the root cause of the
/// spurious-return bug in match statements.
fn fallback_return_atom_from_terminal_stmt(stmts: &[LirStmt]) -> Option<LirAtom> {
    match stmts.last()? {
        LirStmt::IfReturn { then_ret, .. } => then_ret.clone(),
        LirStmt::TryCatch {
            catch_ret,
            body_ret,
            ..
        } => catch_ret.clone().or_else(|| body_ret.clone()),
        _ => None,
    }
}

fn strip_linear(typ: &Type) -> &Type {
    match typ {
        Type::Linear(inner) => strip_linear(inner),
        other => other,
    }
}

/// Produce a default zero-value atom for a given WASM type (used as placeholder).
fn default_atom_for_type(typ: &Type) -> LirAtom {
    match typ {
        Type::I32 | Type::I64 => LirAtom::Int(0),
        Type::F32 | Type::F64 => LirAtom::Float(0.0),
        Type::Bool => LirAtom::Bool(false),
        Type::Char => LirAtom::Char('\0'),
        Type::String => LirAtom::String(String::new()),
        Type::Unit => LirAtom::Unit,
        _ => LirAtom::Int(0), // heap pointer types default to 0
    }
}

/// Collect free variables in a MIR statement block (referenced but not defined).
/// Returns names in a stable order.
fn collect_free_vars(body: &[MirStmt]) -> Vec<Symbol> {
    let mut defined = HashSet::new();
    let mut referenced = Vec::new();
    let mut seen = HashSet::new();
    for stmt in body {
        collect_mir_stmt_refs(stmt, &mut defined, &mut referenced, &mut seen);
    }
    referenced
        .into_iter()
        .filter(|name| !defined.contains(name))
        .collect()
}

fn collect_mir_stmt_refs(
    stmt: &MirStmt,
    defined: &mut HashSet<Symbol>,
    referenced: &mut Vec<Symbol>,
    seen: &mut HashSet<Symbol>,
) {
    match stmt {
        MirStmt::Let { name, expr, .. } => {
            collect_mir_expr_refs(expr, referenced, seen);
            defined.insert(*name);
        }
        MirStmt::Expr(expr) | MirStmt::Return(expr) => {
            collect_mir_expr_refs(expr, referenced, seen);
        }
        MirStmt::Assign { target, value } => {
            collect_mir_expr_refs(target, referenced, seen);
            collect_mir_expr_refs(value, referenced, seen);
        }
        MirStmt::Conc(tasks) => {
            for task in tasks {
                for s in &task.body {
                    collect_mir_stmt_refs(s, defined, referenced, seen);
                }
            }
        }
        MirStmt::Try {
            body,
            catch_param,
            catch_body,
        } => {
            for s in body {
                collect_mir_stmt_refs(s, defined, referenced, seen);
            }
            defined.insert(*catch_param);
            for s in catch_body {
                collect_mir_stmt_refs(s, defined, referenced, seen);
            }
        }
    }
}

fn collect_mir_expr_refs(expr: &MirExpr, referenced: &mut Vec<Symbol>, seen: &mut HashSet<Symbol>) {
    match expr {
        MirExpr::Variable(name) | MirExpr::Borrow(name) => {
            if seen.insert(*name) {
                referenced.push(*name);
            }
        }
        MirExpr::BinaryOp(lhs, _, rhs) => {
            collect_mir_expr_refs(lhs, referenced, seen);
            collect_mir_expr_refs(rhs, referenced, seen);
        }
        MirExpr::Call { args, .. } => {
            for (_, arg) in args {
                collect_mir_expr_refs(arg, referenced, seen);
            }
        }
        MirExpr::Constructor { args, .. } => {
            for (_, arg) in args {
                collect_mir_expr_refs(arg, referenced, seen);
            }
        }
        MirExpr::Record(fields) => {
            for (_, expr) in fields {
                collect_mir_expr_refs(expr, referenced, seen);
            }
        }
        MirExpr::Array(items) => {
            for item in items {
                collect_mir_expr_refs(item, referenced, seen);
            }
        }
        MirExpr::Index(arr, idx) => {
            collect_mir_expr_refs(arr, referenced, seen);
            collect_mir_expr_refs(idx, referenced, seen);
        }
        MirExpr::FieldAccess(expr, _) | MirExpr::Raise(expr) => {
            collect_mir_expr_refs(expr, referenced, seen);
        }
        MirExpr::If {
            cond,
            then_body,
            else_body,
        } => {
            collect_mir_expr_refs(cond, referenced, seen);
            for s in then_body {
                let mut defined = HashSet::new();
                collect_mir_stmt_refs(s, &mut defined, referenced, seen);
            }
            if let Some(else_body) = else_body {
                for s in else_body {
                    let mut defined = HashSet::new();
                    collect_mir_stmt_refs(s, &mut defined, referenced, seen);
                }
            }
        }
        MirExpr::Match { target, cases } => {
            collect_mir_expr_refs(target, referenced, seen);
            for case in cases {
                collect_mir_pattern_defs(&case.pattern, seen);
                for s in &case.body {
                    let mut defined = HashSet::new();
                    collect_mir_stmt_refs(s, &mut defined, referenced, seen);
                }
            }
        }
        MirExpr::While { cond, body } => {
            collect_mir_expr_refs(cond, referenced, seen);
            for s in body {
                let mut defined = HashSet::new();
                collect_mir_stmt_refs(s, &mut defined, referenced, seen);
            }
        }
        MirExpr::FuncRef(_) | MirExpr::Literal(_) => {}
        MirExpr::Closure { captures, .. } => {
            for name in captures {
                if seen.insert(*name) {
                    referenced.push(*name);
                }
            }
        }
        MirExpr::CallIndirect { callee, args, .. } => {
            collect_mir_expr_refs(callee, referenced, seen);
            for (_, arg) in args {
                collect_mir_expr_refs(arg, referenced, seen);
            }
        }
    }
}

fn collect_mir_pattern_defs(pattern: &MirPattern, seen: &mut HashSet<Symbol>) {
    match pattern {
        MirPattern::Variable(name, _) => {
            seen.insert(*name);
        }
        MirPattern::Constructor { fields, .. } => {
            for (_, pat) in fields {
                collect_mir_pattern_defs(pat, seen);
            }
        }
        MirPattern::Record(fields, _) => {
            for (_, pat) in fields {
                collect_mir_pattern_defs(pat, seen);
            }
        }
        MirPattern::Wildcard | MirPattern::Literal(_) => {}
    }
}

/// Resolve the concrete field types for a constructor variant, applying type parameter
/// substitution from the matched enum type.
///
/// For example, matching `Cons(v, rest)` against `List<String>` resolves:
///   v → String, rest → List<String>
fn resolve_constructor_field_types(
    ctor_name: &str,
    matched_type: &Type,
    enum_defs: &[EnumDef],
) -> Option<Vec<Type>> {
    // Extract the enum name and type arguments from the matched type
    let (enum_name, type_args) = match strip_linear(matched_type) {
        Type::UserDefined(name, args) => (name.clone(), args.clone()),
        Type::List(inner) => ("List".to_string(), vec![inner.as_ref().clone()]),
        _ => return None,
    };

    // Find the enum definition (search from end so user defs shadow stdlib)
    let enum_def = enum_defs.iter().rfind(|e| e.name == enum_name)?;

    // Find the variant
    let variant = enum_def.variants.iter().find(|v| v.name == ctor_name)?;

    // Build substitution map: type_param → concrete_type
    let subst: HashMap<String, Type> = enum_def
        .type_params
        .iter()
        .zip(type_args.iter())
        .map(|(param, arg)| (param.clone(), arg.clone()))
        .collect();

    // Apply substitution to each field type
    Some(
        variant
            .fields
            .iter()
            .map(|(_, ft)| apply_type_subst(ft, &subst))
            .collect(),
    )
}

/// Recursively apply a type parameter substitution.
fn apply_type_subst(typ: &Type, subst: &HashMap<String, Type>) -> Type {
    match typ {
        Type::Var(name) => subst.get(name).cloned().unwrap_or_else(|| typ.clone()),
        Type::UserDefined(name, args) => {
            // Check if it's actually a type variable reference with no args
            if args.is_empty() {
                if let Some(concrete) = subst.get(name) {
                    return concrete.clone();
                }
            }
            Type::UserDefined(
                name.clone(),
                args.iter().map(|a| apply_type_subst(a, subst)).collect(),
            )
        }
        Type::List(inner) => Type::List(Box::new(apply_type_subst(inner, subst))),
        Type::Array(inner) => Type::Array(Box::new(apply_type_subst(inner, subst))),
        Type::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(n, t)| (n.clone(), apply_type_subst(t, subst)))
                .collect(),
        ),
        Type::Arrow(params, ret, req, eff) => Type::Arrow(
            params
                .iter()
                .map(|(n, t)| (n.clone(), apply_type_subst(t, subst)))
                .collect(),
            Box::new(apply_type_subst(ret, subst)),
            Box::new(apply_type_subst(req, subst)),
            Box::new(apply_type_subst(eff, subst)),
        ),
        Type::Linear(inner) => Type::Linear(Box::new(apply_type_subst(inner, subst))),
        Type::Row(types, rest) => Type::Row(
            types.iter().map(|t| apply_type_subst(t, subst)).collect(),
            rest.as_ref().map(|r| Box::new(apply_type_subst(r, subst))),
        ),
        Type::Ref(inner) => Type::Ref(Box::new(apply_type_subst(inner, subst))),
        Type::Borrow(inner) => Type::Borrow(Box::new(apply_type_subst(inner, subst))),
        // Primitive types (I32, I64, F32, F64, Bool, String, Unit, etc.): no substitution
        _ => typ.clone(),
    }
}

/// Look up constructor field labels from enum definitions.
/// Returns `Some(vec![...])` where each entry is `Some(label)` or `None` (positional).
/// Returns `None` if the constructor is not found in any enum def.
fn lookup_constructor_field_labels(
    ctor_name: &str,
    enum_defs: &[EnumDef],
) -> Option<Vec<Option<Symbol>>> {
    for def in enum_defs.iter().rev() {
        for variant in &def.variants {
            if variant.name == ctor_name {
                return Some(
                    variant
                        .fields
                        .iter()
                        .map(|(label, _)| label.as_ref().map(|l| Symbol::from(l.as_str())))
                        .collect(),
                );
            }
        }
    }
    None
}

/// Look up the sorted field order for a constructor from enum definitions.
/// Returns a mapping from definition-order index to sorted-order index.
/// Only applies when all fields have labels.
fn constructor_sorted_field_indices(
    ctor_name: &str,
    enum_defs: &[EnumDef],
) -> Option<Vec<usize>> {
    for def in enum_defs.iter().rev() {
        for variant in &def.variants {
            if variant.name == ctor_name {
                let all_labeled = variant.fields.iter().all(|(l, _)| l.is_some());
                if !all_labeled || variant.fields.is_empty() {
                    return None;
                }
                let mut labeled: Vec<(usize, &str)> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, (l, _))| (i, l.as_ref().unwrap().as_str()))
                    .collect();
                labeled.sort_by(|a, b| a.1.cmp(b.1));
                // Build def_idx → sorted_idx mapping
                let mut mapping = vec![0usize; labeled.len()];
                for (sorted_idx, (def_idx, _)) in labeled.iter().enumerate() {
                    mapping[*def_idx] = sorted_idx;
                }
                return Some(mapping);
            }
        }
    }
    None
}

// ────────────────────────────────────────────────────────────────
// Closure conversion: ensure all funcref-target functions have a
// uniform `(__env: i64, params...) -> ret` calling convention.
// ────────────────────────────────────────────────────────────────

/// Collect all function names that are used as FuncRef or Closure targets in the LIR.
fn collect_lir_funcref_targets(functions: &[LirFunction]) -> HashSet<Symbol> {
    let mut targets = HashSet::new();
    fn scan_expr(expr: &LirExpr, targets: &mut HashSet<Symbol>) {
        match expr {
            LirExpr::FuncRef { func, .. } | LirExpr::Closure { func, .. } => {
                targets.insert(*func);
            }
            _ => {}
        }
    }
    fn scan_stmt(stmt: &LirStmt, targets: &mut HashSet<Symbol>) {
        match stmt {
            LirStmt::Let { expr, .. } => scan_expr(expr, targets),
            LirStmt::If { then_body, else_body, .. } => {
                for s in then_body { scan_stmt(s, targets); }
                for s in else_body { scan_stmt(s, targets); }
            }
            LirStmt::IfReturn { then_body, else_body, .. } => {
                for s in then_body { scan_stmt(s, targets); }
                for s in else_body { scan_stmt(s, targets); }
            }
            LirStmt::TryCatch { body, catch_body, .. } => {
                for s in body { scan_stmt(s, targets); }
                for s in catch_body { scan_stmt(s, targets); }
            }
            LirStmt::Conc { .. } => {}
            LirStmt::Loop { cond_stmts, body, .. } => {
                for s in cond_stmts { scan_stmt(s, targets); }
                for s in body { scan_stmt(s, targets); }
            }
        }
    }
    for func in functions {
        for stmt in &func.body { scan_stmt(stmt, &mut targets); }
    }
    targets
}

/// Perform closure conversion on all funcref-target functions.
///
/// - Capturing lambdas: replace capture params with `__env: i64`, prepend ClosureEnvLoad
/// - Non-capturing lambdas: add `__env: i64` as first param (unused)
/// - Named functions: generate a thin wrapper with `__env: i64`
fn closure_convert(
    functions: &mut Vec<LirFunction>,
    mir_functions: &[MirFunction],
) -> Result<(), LirLowerError> {
    let targets = collect_lir_funcref_targets(functions);
    if targets.is_empty() {
        return Ok(());
    }

    let env_sym = Symbol::from("__env");

    // Build a set of lambda function names and their capture info from MIR
    let mir_captures: HashMap<Symbol, &[Symbol]> = mir_functions
        .iter()
        .filter(|f| !f.captures.is_empty())
        .map(|f| (f.name, f.captures.as_slice()))
        .collect();

    // Collect names of functions that need wrappers (non-lambda funcref targets)
    let mut wrapper_targets: Vec<Symbol> = Vec::new();

    for &target in &targets {
        let is_lambda = target.as_str().starts_with("__lambda_");
        if is_lambda {
            // Transform the lambda function in-place
            if let Some(func) = functions.iter_mut().find(|f| f.name == target) {
                if let Some(&captures) = mir_captures.get(&target) {
                    // Capturing lambda: replace capture params with __env, add ClosureEnvLoad preamble
                    let n_captures = captures.len();

                    // Extract capture param types (the first N params)
                    let capture_types: Vec<Type> = func.params[..n_captures]
                        .iter()
                        .map(|p| p.typ.clone())
                        .collect();

                    // Remove capture params, add __env as first
                    func.params = std::iter::once(LirParam {
                        label: env_sym,
                        name: env_sym,
                        typ: Type::I64,
                    })
                    .chain(func.params[n_captures..].iter().cloned())
                    .collect();

                    // Prepend ClosureEnvLoad for each capture
                    let mut preamble: Vec<LirStmt> = captures
                        .iter()
                        .enumerate()
                        .map(|(i, &cap_name)| LirStmt::Let {
                            name: cap_name,
                            typ: wasm_type(&capture_types[i]),
                            expr: LirExpr::ClosureEnvLoad {
                                index: i,
                                typ: capture_types[i].clone(),
                            },
                        })
                        .collect();
                    preamble.append(&mut func.body);
                    func.body = preamble;
                } else {
                    // Non-capturing lambda: just add __env as first param
                    func.params.insert(
                        0,
                        LirParam {
                            label: env_sym,
                            name: env_sym,
                            typ: Type::I64,
                        },
                    );
                }
            }
        } else {
            // Named function: needs a wrapper
            wrapper_targets.push(target);
        }
    }

    // Generate wrapper functions for named funcref targets
    for target in wrapper_targets {
        if let Some(original) = functions.iter().find(|f| f.name == target) {
            let wrapper_name = Symbol::from(format!("__closure_wrap_{}", target));
            let result_sym = Symbol::from("__closure_wrap_result");

            // Wrapper params: __env + original params
            let wrapper_params: Vec<LirParam> = std::iter::once(LirParam {
                label: env_sym,
                name: env_sym,
                typ: Type::I64,
            })
            .chain(original.params.iter().cloned())
            .collect();

            // Build call args from original params (skip __env)
            let call_args: Vec<(Symbol, LirAtom)> = original
                .params
                .iter()
                .map(|p| {
                    (
                        p.label,
                        LirAtom::Var {
                            name: p.name,
                            typ: p.typ.clone(),
                        },
                    )
                })
                .collect();

            let ret_type = original.ret_type.clone();
            let wasm_ret = wasm_type(&ret_type);

            let body = if matches!(wasm_ret, Type::Unit) {
                vec![LirStmt::Let {
                    name: result_sym,
                    typ: wasm_ret.clone(),
                    expr: LirExpr::Call {
                        func: target,
                        args: call_args,
                        typ: wasm_ret.clone(),
                    },
                }]
            } else {
                vec![LirStmt::Let {
                    name: result_sym,
                    typ: wasm_ret.clone(),
                    expr: LirExpr::Call {
                        func: target,
                        args: call_args,
                        typ: wasm_ret.clone(),
                    },
                }]
            };

            let ret_atom = if matches!(wasm_ret, Type::Unit) {
                LirAtom::Unit
            } else {
                LirAtom::Var {
                    name: result_sym,
                    typ: wasm_ret,
                }
            };

            let wrapper = LirFunction {
                name: wrapper_name,
                params: wrapper_params,
                ret_type: ret_type.clone(),
                requires: Type::Row(Vec::new(), None),
                throws: Type::Row(Vec::new(), None),
                body,
                ret: ret_atom,
                span: 0..0,
                source_file: None,
                source_line: None,
            };

            functions.push(wrapper);

            // Update FuncRef nodes: FuncRef(target) → FuncRef(wrapper_name)
            update_funcref_targets(functions, target, wrapper_name);
        }
    }

    Ok(())
}

/// Update all FuncRef nodes for a given target to point to a new name.
fn update_funcref_targets(functions: &mut [LirFunction], old_name: Symbol, new_name: Symbol) {
    fn update_expr(expr: &mut LirExpr, old: Symbol, new: Symbol) {
        match expr {
            LirExpr::FuncRef { func, .. } if *func == old => {
                *func = new;
            }
            _ => {}
        }
    }
    fn update_stmt(stmt: &mut LirStmt, old: Symbol, new: Symbol) {
        match stmt {
            LirStmt::Let { expr, .. } => update_expr(expr, old, new),
            LirStmt::If { then_body, else_body, .. } => {
                for s in then_body { update_stmt(s, old, new); }
                for s in else_body { update_stmt(s, old, new); }
            }
            LirStmt::IfReturn { then_body, else_body, .. } => {
                for s in then_body { update_stmt(s, old, new); }
                for s in else_body { update_stmt(s, old, new); }
            }
            LirStmt::TryCatch { body, catch_body, .. } => {
                for s in body { update_stmt(s, old, new); }
                for s in catch_body { update_stmt(s, old, new); }
            }
            LirStmt::Conc { .. } => {}
            LirStmt::Loop { cond_stmts, body, .. } => {
                for s in cond_stmts { update_stmt(s, old, new); }
                for s in body { update_stmt(s, old, new); }
            }
        }
    }
    for func in functions.iter_mut() {
        for stmt in &mut func.body {
            update_stmt(stmt, old_name, new_name);
        }
    }
}
