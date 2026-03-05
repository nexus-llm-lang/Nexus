//! MIR → LIR (ANF conversion)
//!
//! Flattens complex MIR expressions into ANF form:
//! - All operands become atoms (variables or literals)
//! - Complex expressions are extracted into let-bound temporaries
//! - If/Match compiled into IfReturn chains

use crate::ir::lir::*;
use crate::ir::mir::*;
use crate::lang::ast::{BinaryOp, Literal, Type};
use std::collections::HashMap;

#[derive(Debug)]
pub enum LirLowerError {
    UnsupportedExpression { detail: String },
    FunctionMayNotReturn { name: String },
}

impl std::fmt::Display for LirLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LirLowerError::UnsupportedExpression { detail } => {
                write!(f, "Unsupported expression in LIR lowering: {}", detail)
            }
            LirLowerError::FunctionMayNotReturn { name } => {
                write!(f, "Function '{}' may not return a value", name)
            }
        }
    }
}

#[tracing::instrument(skip_all, name = "lower_mir_to_lir")]
pub fn lower_mir_to_lir(mir: &MirProgram) -> Result<LirProgram, LirLowerError> {
    let mut lowerer = LirLowerer::new(mir);
    lowerer.lower()
}

struct LirLowerer<'a> {
    mir: &'a MirProgram,
}

impl<'a> LirLowerer<'a> {
    fn new(mir: &'a MirProgram) -> Self {
        LirLowerer { mir }
    }

    fn lower(&mut self) -> Result<LirProgram, LirLowerError> {
        let mut functions = Vec::new();
        for func in &self.mir.functions {
            functions.push(self.lower_function(func)?);
        }

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
                effects: ext.effects.clone(),
            })
            .collect();

        Ok(LirProgram {
            functions,
            externals,
        })
    }

    fn lower_function(&self, func: &MirFunction) -> Result<LirFunction, LirLowerError> {
        let mut ctx = LowerCtx::new();

        // Register params in vars (both wasm and semantic types)
        for p in &func.params {
            ctx.vars.insert(p.name.clone(), wasm_type(&p.typ));
            ctx.semantic_vars.insert(p.name.clone(), p.typ.clone());
        }
        // Register evidence params as i32 variables
        for ep in &func.evidence_params {
            ctx.vars.insert(ep.param_name.clone(), Type::I32);
            ctx.semantic_vars.insert(ep.param_name.clone(), Type::I32);
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
            return Err(LirLowerError::FunctionMayNotReturn {
                name: func.name.clone(),
            });
        };

        let params: Vec<LirParam> = func
            .params
            .iter()
            .map(|p| LirParam {
                label: p.label.clone(),
                name: p.name.clone(),
                typ: p.typ.clone(),
            })
            .collect();

        let evidence_params: Vec<LirParam> = func
            .evidence_params
            .iter()
            .map(|ep| LirParam {
                label: ep.param_name.clone(),
                name: ep.param_name.clone(),
                typ: Type::I32,
            })
            .collect();

        Ok(LirFunction {
            name: func.name.clone(),
            params,
            evidence_params,
            ret_type: func.ret_type.clone(),
            requires: Type::Row(Vec::new(), None), // evidence already passed
            effects: Type::Row(Vec::new(), None),
            body: ctx.stmts,
            ret,
        })
    }
}

/// Context for lowering a single function body
struct LowerCtx {
    vars: HashMap<String, Type>,
    /// Semantic types for variables (pre-wasm-lowering) — used for field access resolution
    semantic_vars: HashMap<String, Type>,
    stmts: Vec<LirStmt>,
    temp_counter: usize,
}

impl LowerCtx {
    fn new() -> Self {
        LowerCtx {
            vars: HashMap::new(),
            semantic_vars: HashMap::new(),
            stmts: Vec::new(),
            temp_counter: 0,
        }
    }

    fn new_temp(&mut self) -> String {
        let name = format!("__t{}", self.temp_counter);
        self.temp_counter += 1;
        name
    }

    /// Bind a complex expression to a temporary variable, returning an atom reference
    fn bind_expr_to_temp(&mut self, expr: LirExpr, typ: Type) -> LirAtom {
        let name = self.new_temp();
        self.vars.insert(name.clone(), typ.clone());
        self.stmts.push(LirStmt::Let {
            name: name.clone(),
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
                let semantic_type = {
                    let expr_inferred = self.infer_semantic_type(expr);
                    if matches!(typ, Type::Unit) || matches!(typ, Type::I64) {
                        expr_inferred
                    } else {
                        typ.clone()
                    }
                };
                let atom = self.lower_expr_to_atom(expr)?;
                let inferred = atom.typ();
                self.vars.insert(name.clone(), inferred.clone());
                self.semantic_vars.insert(name.clone(), semantic_type);
                self.stmts.push(LirStmt::Let {
                    name: name.clone(),
                    typ: inferred,
                    expr: LirExpr::Atom(atom),
                });
                Ok(None)
            }
            MirStmt::Expr(expr) => {
                match expr {
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
                    _ => {
                        let _atom = self.lower_expr_to_atom(expr)?;
                        Ok(None)
                    }
                }
            }
            MirStmt::Return(expr) => {
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
            MirStmt::Conc(_tasks) => {
                // Concurrency not yet supported in LIR
                // TODO: implement when runtime support is ready
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
                let (catch_stmts, catch_ret) = self.lower_block_with_vars(
                    catch_body,
                    ret_type,
                    catch_vars,
                    catch_sem_vars,
                )?;

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
                    then_ret: then_ret.unwrap_or(LirAtom::Unit),
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
                then_ret: then_ret.unwrap_or(LirAtom::Unit),
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

    /// Lower a match expression into a chain of IfReturn statements
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

        // Build chain: each case becomes an IfReturn with cond=true for the last (fallback) case
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

            let then_ret = case_ret.unwrap_or(LirAtom::Unit);

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

    /// Lower a single match case, returning (condition, body_stmts, return_atom)
    fn lower_match_case(
        &mut self,
        target: &LirAtom,
        pattern: &MirPattern,
        body: &[MirStmt],
        ret_type: &Type,
    ) -> Result<(Option<LirAtom>, Vec<LirStmt>, Option<LirAtom>), LirLowerError> {
        let mut conds = Vec::new();
        let mut bindings: HashMap<String, LirAtom> = HashMap::new();

        self.collect_pattern_conditions_and_bindings(
            target,
            pattern,
            &mut conds,
            &mut bindings,
        )?;

        // Combine conditions with And
        let combined_cond = if conds.is_empty() {
            None
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

        let (mut case_stmts, case_ret) =
            self.lower_block_with_vars(body, ret_type, case_vars, case_sem_vars)?;

        // Prepend binding let-statements
        let mut binding_stmts: Vec<LirStmt> = Vec::new();
        for (name, atom) in bindings {
            let typ = atom.typ();
            binding_stmts.push(LirStmt::Let {
                name,
                typ: typ.clone(),
                expr: LirExpr::Atom(atom),
            });
        }
        binding_stmts.append(&mut case_stmts);

        Ok((combined_cond, binding_stmts, case_ret))
    }

    /// Collect pattern matching conditions and variable bindings
    fn collect_pattern_conditions_and_bindings(
        &mut self,
        target: &LirAtom,
        pattern: &MirPattern,
        conds: &mut Vec<LirAtom>,
        bindings: &mut HashMap<String, LirAtom>,
    ) -> Result<(), LirLowerError> {
        match pattern {
            MirPattern::Wildcard => {} // always matches
            MirPattern::Variable(name, _sigil) => {
                bindings.insert(name.clone(), target.clone());
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
                // Tag check
                let tag_atom = self.bind_expr_to_temp(
                    LirExpr::ObjectTag {
                        value: target.clone(),
                        typ: Type::I64,
                    },
                    Type::I64,
                );
                let expected_tag = constructor_tag(name, fields.len());
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

                // Field checks
                for (idx, (_label, field_pat)) in fields.iter().enumerate() {
                    let field_atom = self.bind_expr_to_temp(
                        LirExpr::ObjectField {
                            value: target.clone(),
                            index: idx,
                            typ: Type::I64, // generic object field type
                        },
                        Type::I64,
                    );
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
                        .unwrap_or_else(|| {
                            // Fallback: use pattern order index (legacy behavior)
                            let idx = fields.iter().position(|(n, _)| n == name).unwrap_or(0);
                            (idx, Type::I64)
                        });
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
    ) -> HashMap<String, Type> {
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
        out: &mut HashMap<String, Type>,
    ) {
        match pattern {
            MirPattern::Variable(name, _) => {
                out.insert(name.clone(), sem_type.clone());
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
                        .find(|(n, _)| n == name)
                        .map(|(_, t)| t.clone())
                        .unwrap_or(Type::I64);
                    self.walk_pattern_semantic_types(&field_type, field_pat, out);
                }
            }
            MirPattern::Constructor { fields, .. } => {
                // Constructor fields don't have semantic types tracked yet
                for (_, field_pat) in fields {
                    self.walk_pattern_semantic_types(&Type::I64, field_pat, out);
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
                let typ = self
                    .vars
                    .get(name)
                    .cloned()
                    .unwrap_or(Type::I64); // fallback type
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
                evidence_args,
                ret_type,
            } => {
                let mut lir_args: Vec<(String, LirAtom)> = Vec::new();
                for (label, expr) in args {
                    let atom = self.lower_expr_to_atom(expr)?;
                    lir_args.push((label.clone(), atom));
                }
                // Append evidence args as additional arguments
                for (idx, ev_arg) in evidence_args.iter().enumerate() {
                    let atom = self.lower_expr_to_atom(ev_arg)?;
                    lir_args.push((format!("__ev_{}", idx), atom));
                }
                let typ = wasm_type(ret_type);
                Ok(self.bind_expr_to_temp(
                    LirExpr::Call {
                        func: func.clone(),
                        args: lir_args,
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::Constructor { name, args } => {
                let mut lir_args = Vec::new();
                for arg in args {
                    lir_args.push(self.lower_expr_to_atom(arg)?);
                }
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
                    lir_items.push((idx.to_string(), atom));
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
                        func: "__array_get".to_string(),
                        args: vec![
                            ("arr".to_string(), arr_atom),
                            ("idx".to_string(), idx_atom),
                        ],
                        typ: typ.clone(),
                    },
                    typ,
                ))
            }
            MirExpr::FieldAccess(expr, field) => {
                // Resolve the receiver's semantic type to determine field index and type
                let receiver_semantic_type = self.infer_semantic_type(expr);
                let obj_atom = self.lower_expr_to_atom(expr)?;

                let (idx, field_type) =
                    resolve_field_access(&receiver_semantic_type, field);

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
                cond: _,
                then_body: _,
                else_body: _,
            } => {
                // If used as expression — lower to IfReturn
                Err(LirLowerError::UnsupportedExpression {
                    detail: "If expression in atom position; should be lowered at statement level"
                        .to_string(),
                })
            }
            MirExpr::Match { target: _, cases: _ } => {
                Err(LirLowerError::UnsupportedExpression {
                    detail:
                        "Match expression in atom position; should be lowered at statement level"
                            .to_string(),
                })
            }
            MirExpr::Borrow(name) => {
                let typ = self.vars.get(name).cloned().unwrap_or(Type::I64);
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
        }
    }

    /// Infer the semantic (pre-wasm) type of a MIR expression by looking up
    /// variable bindings in semantic_vars.
    fn infer_semantic_type(&self, expr: &MirExpr) -> Type {
        match expr {
            MirExpr::Variable(name) => self
                .semantic_vars
                .get(name)
                .cloned()
                .unwrap_or(Type::I64),
            MirExpr::Call { ret_type, .. } => ret_type.clone(),
            MirExpr::Record(fields) => {
                let mut field_types: Vec<(String, Type)> = fields
                    .iter()
                    .map(|(name, expr)| (name.clone(), self.infer_semantic_type(expr)))
                    .collect();
                field_types.sort_by(|a, b| a.0.cmp(&b.0));
                Type::Record(field_types)
            }
            MirExpr::Constructor { .. } => Type::I64,
            MirExpr::Literal(lit) => match lit {
                Literal::Int(_) => Type::I64,
                Literal::Float(_) => Type::F64,
                Literal::Bool(_) => Type::Bool,
                Literal::String(_) => Type::String,
                Literal::Unit => Type::Unit,
            },
            _ => Type::I64,
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
        vars: HashMap<String, Type>,
        semantic_vars: HashMap<String, Type>,
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
        Literal::String(s) => LirAtom::String(s.clone()),
        Literal::Unit => LirAtom::Unit,
    }
}

/// Resolve a field access on a semantic type.
/// Returns (field_index, field_type).
/// Fields are sorted alphabetically (matching record layout in codegen).
fn resolve_field_access(receiver_type: &Type, field_name: &str) -> (usize, Type) {
    match receiver_type {
        Type::Record(fields) => {
            let mut sorted: Vec<(String, Type)> = fields.clone();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (idx, (name, typ)) in sorted.iter().enumerate() {
                if name == field_name {
                    return (idx, typ.clone());
                }
            }
            // Field not found — fallback
            (0, Type::I64)
        }
        _ => {
            // Can't resolve field on non-record — fallback
            (0, Type::I64)
        }
    }
}

/// Map a high-level AST type to its WASM-level representation.
/// Records, enums, arrays, and other heap-allocated types become I64 (object pointer).
/// Primitives pass through unchanged.
fn wasm_type(typ: &Type) -> Type {
    match typ {
        Type::I32 | Type::I64 | Type::F32 | Type::F64 | Type::Bool | Type::String | Type::Unit => {
            typ.clone()
        }
        Type::IntLit => Type::I64,
        Type::FloatLit => Type::F64,
        // Heap-allocated compound types → I64 (object pointer)
        Type::Record(_)
        | Type::UserDefined(_, _)
        | Type::Array(_)
        | Type::List(_)
        | Type::Linear(_)
        | Type::Borrow(_)
        | Type::Ref(_)
        | Type::Handler(_, _) => Type::I64,
        // Function types → I64 (funcref / closure pointer)
        Type::Arrow(_, _, _, _) => Type::I64,
        // Rows are not values
        Type::Row(_, _) => Type::Unit,
        // Type variables (generics) → I64 as fallback
        Type::Var(_) => Type::I64,
    }
}

/// Compute constructor tag (FNV-1a-like hash of name + arity)
fn constructor_tag(name: &str, arity: usize) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= arity as u64;
    hash = hash.wrapping_mul(0x100000001b3);
    hash as i64
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

/// Try to extract a return atom from the last statement in a block
fn fallback_return_atom_from_terminal_stmt(stmts: &[LirStmt]) -> Option<LirAtom> {
    match stmts.last()? {
        LirStmt::IfReturn { then_ret, .. } => Some(then_ret.clone()),
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
