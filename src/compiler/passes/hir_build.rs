//! AST → HIR (module flatten, name resolve, handler collect)
//!
//! Extracts and restructures logic from lower.rs:
//! - Module resolution (imports, file loading, cycle detection)
//! - Name resolution (qualify identifiers, resolve constructors)
//! - Handler collection (synthesize handler functions)
//! - Port registration
//! - Reachability analysis
//! - External binding resolution

use crate::ir::hir::*;
use crate::lang::ast::*;
use crate::lang::stdlib::load_stdlib_nx_programs;
use crate::lang::parser;
use std::collections::{HashMap, HashSet};
use std::fs;

#[derive(Debug)]
pub enum HirBuildError {
    ImportReadError { path: String, detail: String },
    ImportParseError { path: String, detail: String },
    CyclicImport { path: String },
    ImportItemNotFound { item: String, path: String },
}

impl std::fmt::Display for HirBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HirBuildError::ImportReadError { path, detail } => {
                write!(f, "Failed to read '{}': {}", path, detail)
            }
            HirBuildError::ImportParseError { path, detail } => {
                write!(f, "Failed to parse '{}': {}", path, detail)
            }
            HirBuildError::CyclicImport { path } => {
                write!(f, "Cyclic import detected: {}", path)
            }
            HirBuildError::ImportItemNotFound { item, path } => {
                write!(f, "Item '{}' not found in '{}'", item, path)
            }
        }
    }
}

/// Builds an HirProgram from a parsed AST Program.
/// Performs module resolution, name resolution, and handler collection.
#[tracing::instrument(skip_all, name = "build_hir")]
pub fn build_hir(program: &Program) -> Result<HirProgram, HirBuildError> {
    let mut builder = HirBuilder::new();
    builder.build(program)
}

struct HirBuilder {
    ports: Vec<HirPort>,
    functions: Vec<HirFunction>,
    externals: Vec<HirExternal>,
    handler_bindings: HashMap<String, HirHandlerBinding>,
    import_stack: Vec<String>,
}

impl HirBuilder {
    fn new() -> Self {
        HirBuilder {
            ports: Vec::new(),
            functions: Vec::new(),
            externals: Vec::new(),
            handler_bindings: HashMap::new(),
            import_stack: Vec::new(),
        }
    }

    fn build(&mut self, program: &Program) -> Result<HirProgram, HirBuildError> {
        // Load stdlib definitions (enums, exceptions, externals)
        self.load_stdlib()?;

        // Flatten all modules into this builder
        self.process_program(program, &HashMap::new())?;

        // Collect reachable functions from main
        let reachable = self.collect_reachable();

        // Filter to only reachable functions and externals
        let functions: Vec<HirFunction> = self
            .functions
            .iter()
            .filter(|f| reachable.contains(&f.name))
            .cloned()
            .collect();
        let externals: Vec<HirExternal> = self
            .externals
            .iter()
            .filter(|e| reachable.contains(&e.name))
            .cloned()
            .collect();
        // Filter handler bindings: keep only bindings that are reachable,
        // and within each binding, keep only reachable handler functions
        let handler_bindings: HashMap<String, HirHandlerBinding> = self
            .handler_bindings
            .iter()
            .filter(|(name, _)| {
                self.handler_bindings.get(*name).map_or(false, |b| {
                    b.functions
                        .iter()
                        .any(|f| reachable.contains(&f.name))
                }) || reachable.contains(name.as_str())
            })
            .map(|(k, v)| {
                let filtered_fns = v
                    .functions
                    .iter()
                    .filter(|f| reachable.contains(&f.name))
                    .cloned()
                    .collect();
                (
                    k.clone(),
                    HirHandlerBinding {
                        port_name: v.port_name.clone(),
                        functions: filtered_fns,
                    },
                )
            })
            .collect();

        Ok(HirProgram {
            ports: self.ports.clone(),
            functions,
            externals,
            handler_bindings,
        })
    }

    /// Collect all function names transitively reachable from "main"
    fn collect_reachable(&self) -> HashSet<String> {
        let mut reachable = HashSet::new();
        let mut worklist: Vec<String> = vec!["main".to_string()];

        while let Some(name) = worklist.pop() {
            if !reachable.insert(name.clone()) {
                continue;
            }
            // Find function body
            if let Some(func) = self.functions.iter().find(|f| f.name == name) {
                let mut calls = Vec::new();
                collect_calls_in_hir_stmts(&func.body, &mut calls);
                for called in calls {
                    // Resolve port calls to handler functions
                    if let Some((port, method)) = called.split_once('.') {
                        let mut resolved_as_port = false;
                        for (binding_name, binding) in &self.handler_bindings {
                            if binding.port_name == port {
                                let handler_fn =
                                    format!("__handler_{}_{}", binding_name, method);
                                if !reachable.contains(&handler_fn) {
                                    worklist.push(handler_fn);
                                }
                                resolved_as_port = true;
                            }
                        }
                        // If not a port call, treat as a module-qualified function name
                        if !resolved_as_port && !reachable.contains(&called) {
                            worklist.push(called);
                        }
                    } else if !reachable.contains(&called) {
                        worklist.push(called);
                    }
                }
            }
            // Check handler functions for calls within inject blocks
            for binding in self.handler_bindings.values() {
                for func in &binding.functions {
                    if func.name == name {
                        let mut calls = Vec::new();
                        collect_calls_in_hir_stmts(&func.body, &mut calls);
                        for called in calls {
                            if !reachable.contains(&called) {
                                worklist.push(called);
                            }
                        }
                    }
                }
            }
        }
        reachable
    }

    fn load_stdlib(&mut self) -> Result<(), HirBuildError> {
        if let Ok(stdlib_programs) = load_stdlib_nx_programs() {
            for (_, stdlib_program) in stdlib_programs {
                let mut current_wasm_module: Option<String> = None;
                for def in &stdlib_program.definitions {
                    match &def.node {
                        TopLevel::Import(import) if import.is_external => {
                            current_wasm_module = Some(import.path.clone());
                        }
                        TopLevel::Enum(_) => {}
                        TopLevel::Exception(_) => {}
                        TopLevel::Let(gl) if gl.is_public => {
                            if let Expr::External(wasm_name, _type_params, typ) = &gl.value.node {
                                if let Type::Arrow(params, ret, _requires, effects) = typ {
                                    if let Some(ref wasm_mod) = current_wasm_module {
                                        // Skip if already loaded (avoid duplicates)
                                        if self.externals.iter().any(|e| e.name == gl.name) {
                                            continue;
                                        }
                                        self.externals.push(HirExternal {
                                            name: gl.name.clone(),
                                            wasm_module: wasm_mod.clone(),
                                            wasm_name: wasm_name.clone(),
                                            params: params
                                                .iter()
                                                .map(|(n, t)| HirParam {
                                                    name: n.clone(),
                                                    label: n.clone(),
                                                    typ: t.clone(),
                                                })
                                                .collect(),
                                            ret_type: *ret.clone(),
                                            effects: *effects.clone(),
                                        });
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn process_program(
        &mut self,
        program: &Program,
        rename_map: &HashMap<String, String>,
    ) -> Result<(), HirBuildError> {
        let mut current_wasm_module: Option<String> = None;

        for def in &program.definitions {
            match &def.node {
                TopLevel::Import(import) if import.is_external => {
                    current_wasm_module = Some(import.path.clone());
                }
                TopLevel::Import(import) => {
                    self.process_import(import)?;
                }
                TopLevel::TypeDef(_) => {}
                TopLevel::Enum(_) => {}
                TopLevel::Exception(_) => {}
                TopLevel::Port(port) => {
                    self.ports.push(HirPort {
                        name: self.rename(&port.name, rename_map),
                        functions: port.functions.iter().map(|f| HirPortMethod {
                            name: f.name.clone(),
                        }).collect(),
                    });
                }
                TopLevel::Let(gl) => {
                    let name = self.rename(&gl.name, rename_map);
                    match &gl.value.node {
                        Expr::Lambda {
                            type_params: _,
                            params,
                            ret_type,
                            requires: _,
                            effects: _,
                            body,
                        } => {
                            self.functions.push(HirFunction {
                                name: name.clone(),
                                params: params.iter().map(|p| HirParam {
                                    name: p.name.clone(),
                                    label: p.name.clone(),
                                    typ: p.typ.clone(),
                                }).collect(),
                                ret_type: ret_type.clone(),
                                body: self.convert_stmts(body, rename_map),
                            });
                        }
                        Expr::External(wasm_name, _type_params, typ) => {
                            if let Type::Arrow(params, ret, _requires, effects) = typ {
                                if let Some(ref wasm_mod) = current_wasm_module {
                                    // If already defined, replace it (explicit import overrides auto-loaded)
                                    self.externals.retain(|e| e.name != name);
                                    self.externals.push(HirExternal {
                                        name: name.clone(),
                                        wasm_module: wasm_mod.clone(),
                                        wasm_name: wasm_name.clone(),
                                        params: params.iter().map(|(n, t)| HirParam {
                                            name: n.clone(),
                                            label: n.clone(),
                                            typ: t.clone(),
                                        }).collect(),
                                        ret_type: *ret.clone(),
                                        effects: *effects.clone(),
                                    });
                                }
                            }
                        }
                        Expr::Handler {
                            coeffect_name,
                            requires: _,
                            functions: handler_fns,
                        } => {
                            let mut hir_fns = Vec::new();
                            for hf in handler_fns {
                                let synth_name = format!("__handler_{}_{}", name, hf.name);
                                hir_fns.push(HirFunction {
                                    name: synth_name,
                                    params: hf.params.iter().map(|p| HirParam {
                                        name: p.name.clone(),
                                        label: p.name.clone(),
                                        typ: p.typ.clone(),
                                    }).collect(),
                                    ret_type: hf.ret_type.clone(),
                                    body: self.convert_stmts(&hf.body, rename_map),
                                });
                            }
                            self.handler_bindings.insert(
                                name.clone(),
                                HirHandlerBinding {
                                    port_name: self.rename(coeffect_name, rename_map),
                                    functions: hir_fns,
                                },
                            );
                        }
                        _ => {
                            // Global let values not represented in HIR
                            // (no downstream consumer reads them)
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn process_import(&mut self, import: &Import) -> Result<(), HirBuildError> {
        if self.import_stack.iter().any(|p| p == &import.path) {
            return Err(HirBuildError::CyclicImport {
                path: import.path.clone(),
            });
        }

        self.import_stack.push(import.path.clone());
        let src = fs::read_to_string(&import.path).map_err(|e| {
            HirBuildError::ImportReadError {
                path: import.path.clone(),
                detail: e.to_string(),
            }
        })?;
        let imported_program = parser::parser().parse(&src).map_err(|e| {
            HirBuildError::ImportParseError {
                path: import.path.clone(),
                detail: format!("{:?}", e),
            }
        })?;

        let rename_map = self.build_rename_map(&imported_program, import)?;
        let result = self.process_program(&imported_program, &rename_map);
        self.import_stack.pop();
        result
    }

    fn build_rename_map(
        &self,
        program: &Program,
        import: &Import,
    ) -> Result<HashMap<String, String>, HirBuildError> {
        let mut map = HashMap::new();

        if import.items.is_empty() {
            // Wildcard import: prefix everything with alias
            let alias = import
                .alias
                .clone()
                .unwrap_or_else(|| get_default_alias(&import.path));
            for def in &program.definitions {
                if let TopLevel::Let(gl) = &def.node {
                    map.insert(gl.name.clone(), format!("{}.{}", alias, gl.name));
                }
            }
            return Ok(map);
        }

        // Selective import
        let selected: HashSet<String> = import.items.iter().cloned().collect();
        for item in &import.items {
            let found = program.definitions.iter().any(|def| match &def.node {
                TopLevel::Let(gl) if gl.is_public && gl.name == *item => true,
                TopLevel::Port(port) if port.is_public && port.name == *item => true,
                TopLevel::Enum(ed) if ed.is_public && ed.name == *item => true,
                TopLevel::TypeDef(td) if td.is_public && td.name == *item => true,
                _ => false,
            });
            if !found {
                return Err(HirBuildError::ImportItemNotFound {
                    item: item.clone(),
                    path: import.path.clone(),
                });
            }
        }

        if let Some(alias) = &import.alias {
            for def in &program.definitions {
                if let TopLevel::Let(gl) = &def.node {
                    if gl.is_public && selected.contains(&gl.name) {
                        map.insert(gl.name.clone(), gl.name.clone());
                    } else {
                        map.insert(gl.name.clone(), format!("{}.{}", alias, gl.name));
                    }
                }
            }
        } else {
            let alias_prefix = format!("__import_{}", self.import_stack.len());
            for def in &program.definitions {
                if let TopLevel::Let(gl) = &def.node {
                    if selected.contains(&gl.name) {
                        map.insert(gl.name.clone(), gl.name.clone());
                    } else {
                        map.insert(
                            gl.name.clone(),
                            format!("{}_{}", alias_prefix, gl.name),
                        );
                    }
                }
            }
        }

        Ok(map)
    }

    fn rename(&self, name: &str, map: &HashMap<String, String>) -> String {
        map.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    // ---- AST → HIR conversion helpers ----

    fn convert_stmts(&self, stmts: &[Spanned<Stmt>], rename_map: &HashMap<String, String>) -> Vec<HirStmt> {
        stmts.iter().filter_map(|s| self.convert_stmt(&s.node, rename_map)).collect()
    }

    fn convert_stmt(&self, stmt: &Stmt, rename_map: &HashMap<String, String>) -> Option<HirStmt> {
        match stmt {
            Stmt::Let { name, typ, value, .. } => {
                Some(HirStmt::Let {
                    name: name.clone(),
                    typ: typ.clone(),
                    value: self.convert_expr(value, rename_map),
                })
            }
            Stmt::Expr(expr) => {
                Some(HirStmt::Expr(self.convert_expr(expr, rename_map)))
            }
            Stmt::Return(expr) => {
                Some(HirStmt::Return(self.convert_expr(expr, rename_map)))
            }
            Stmt::Assign { target, value } => {
                Some(HirStmt::Assign {
                    target: self.convert_expr(target, rename_map),
                    value: self.convert_expr(value, rename_map),
                })
            }
            Stmt::Conc(tasks) => {
                let hir_tasks: Vec<HirFunction> = tasks.iter().map(|t| {
                    HirFunction {
                        name: t.name.clone(),
                        params: vec![],
                        ret_type: Type::Unit,
                        body: self.convert_stmts(&t.body, rename_map),
                    }
                }).collect();
                Some(HirStmt::Conc(hir_tasks))
            }
            Stmt::Try { body, catch_param, catch_body } => {
                Some(HirStmt::Try {
                    body: self.convert_stmts(body, rename_map),
                    catch_param: catch_param.clone(),
                    catch_body: self.convert_stmts(catch_body, rename_map),
                })
            }
            Stmt::Inject { handlers, body } => {
                Some(HirStmt::Inject {
                    handlers: handlers.iter().map(|h| self.rename(h, rename_map)).collect(),
                    body: self.convert_stmts(body, rename_map),
                })
            }
        }
    }

    fn convert_expr(&self, expr: &Spanned<Expr>, rename_map: &HashMap<String, String>) -> HirExpr {
        match &expr.node {
            Expr::Literal(lit) => HirExpr::Literal(lit.clone()),
            Expr::Variable(name, sigil) => {
                HirExpr::Variable(self.rename(name, rename_map), sigil.clone())
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                HirExpr::BinaryOp(
                    Box::new(self.convert_expr(lhs, rename_map)),
                    *op,
                    Box::new(self.convert_expr(rhs, rename_map)),
                )
            }
            Expr::Borrow(name, sigil) => {
                HirExpr::Borrow(self.rename(name, rename_map), sigil.clone())
            }
            Expr::Call { func, args } => {
                let renamed_func = self.rename(func, rename_map);
                HirExpr::Call {
                    func: renamed_func,
                    args: args.iter().map(|(n, e)| {
                        (n.clone(), self.convert_expr(e, rename_map))
                    }).collect(),
                }
            }
            Expr::Constructor(name, args) => {
                HirExpr::Constructor {
                    variant: name.clone(),
                    args: args.iter().map(|(_, e)| self.convert_expr(e, rename_map)).collect(),
                }
            }
            Expr::Record(fields) => {
                HirExpr::Record(fields.iter().map(|(n, e)| {
                    (n.clone(), self.convert_expr(e, rename_map))
                }).collect())
            }
            Expr::Array(items) => {
                HirExpr::Array(items.iter().map(|e| self.convert_expr(e, rename_map)).collect())
            }
            Expr::List(items) => {
                // Desugar [a, b, c] → Cons(a, Cons(b, Cons(c, Nil)))
                let mut acc = HirExpr::Constructor {
                    variant: "Nil".to_string(),
                    args: vec![],
                };
                for item in items.iter().rev() {
                    acc = HirExpr::Constructor {
                        variant: "Cons".to_string(),
                        args: vec![self.convert_expr(item, rename_map), acc],
                    };
                }
                acc
            }
            Expr::Index(arr, idx) => {
                HirExpr::Index(
                    Box::new(self.convert_expr(arr, rename_map)),
                    Box::new(self.convert_expr(idx, rename_map)),
                )
            }
            Expr::FieldAccess(expr, field) => {
                HirExpr::FieldAccess(
                    Box::new(self.convert_expr(expr, rename_map)),
                    field.clone(),
                )
            }
            Expr::If { cond, then_branch, else_branch } => {
                HirExpr::If {
                    cond: Box::new(self.convert_expr(cond, rename_map)),
                    then_branch: self.convert_stmts(then_branch, rename_map),
                    else_branch: else_branch.as_ref().map(|b| self.convert_stmts(b, rename_map)),
                }
            }
            Expr::Match { target, cases } => {
                HirExpr::Match {
                    target: Box::new(self.convert_expr(target, rename_map)),
                    cases: cases.iter().map(|c| HirMatchCase {
                        pattern: self.convert_pattern(&c.pattern.node),
                        body: self.convert_stmts(&c.body, rename_map),
                    }).collect(),
                }
            }
            Expr::Lambda { type_params: _, params, ret_type, requires: _, effects: _, body } => {
                HirExpr::Lambda {
                    params: params.iter().map(|p| HirParam {
                        name: p.name.clone(),
                        label: p.name.clone(),
                        typ: p.typ.clone(),
                    }).collect(),
                    ret_type: ret_type.clone(),
                    body: self.convert_stmts(body, rename_map),
                }
            }
            Expr::Raise(expr) => {
                HirExpr::Raise(Box::new(self.convert_expr(expr, rename_map)))
            }
            Expr::External(sym, tparams, typ) => {
                HirExpr::External(sym.clone(), tparams.clone(), typ.clone())
            }
            Expr::Handler { coeffect_name: _, requires: _, functions } => {
                HirExpr::Handler {
                    functions: functions.iter().map(|f| HirFunction {
                        name: f.name.clone(),
                        params: f.params.iter().map(|p| HirParam {
                            name: p.name.clone(),
                            label: p.name.clone(),
                            typ: p.typ.clone(),
                        }).collect(),
                        ret_type: f.ret_type.clone(),
                        body: self.convert_stmts(&f.body, rename_map),
                    }).collect(),
                }
            }
        }
    }

    fn convert_pattern(&self, pattern: &Pattern) -> HirPattern {
        match pattern {
            Pattern::Literal(lit) => HirPattern::Literal(lit.clone()),
            Pattern::Variable(name, sigil) => HirPattern::Variable(name.clone(), sigil.clone()),
            Pattern::Constructor(name, fields) => {
                HirPattern::Constructor {
                    variant: name.clone(),
                    fields: fields.iter().map(|(label, p)| {
                        (label.clone(), self.convert_pattern(&p.node))
                    }).collect(),
                }
            }
            Pattern::Record(fields, open) => {
                HirPattern::Record(
                    fields.iter().map(|(n, p)| (n.clone(), self.convert_pattern(&p.node))).collect(),
                    *open,
                )
            }
            Pattern::Wildcard => HirPattern::Wildcard,
        }
    }

}

fn get_default_alias(path: &str) -> String {
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .split('.')
        .next()
        .unwrap_or(path);
    stem.to_string()
}

/// Collect function names called in HIR statements
fn collect_calls_in_hir_stmts(stmts: &[HirStmt], out: &mut Vec<String>) {
    for stmt in stmts {
        match stmt {
            HirStmt::Let { value, .. } => collect_calls_in_hir_expr(value, out),
            HirStmt::Expr(expr) => collect_calls_in_hir_expr(expr, out),
            HirStmt::Return(expr) => collect_calls_in_hir_expr(expr, out),
            HirStmt::Assign { target, value } => {
                collect_calls_in_hir_expr(target, out);
                collect_calls_in_hir_expr(value, out);
            }
            HirStmt::Inject { handlers, body } => {
                out.extend(handlers.iter().cloned());
                collect_calls_in_hir_stmts(body, out);
            }
            HirStmt::Conc(tasks) => {
                for task in tasks {
                    collect_calls_in_hir_stmts(&task.body, out);
                }
            }
            HirStmt::Try {
                body,
                catch_body,
                ..
            } => {
                collect_calls_in_hir_stmts(body, out);
                collect_calls_in_hir_stmts(catch_body, out);
            }
        }
    }
}

fn collect_calls_in_hir_expr(expr: &HirExpr, out: &mut Vec<String>) {
    match expr {
        HirExpr::Call { func, args } => {
            out.push(func.clone());
            for (_, arg) in args {
                collect_calls_in_hir_expr(arg, out);
            }
        }
        HirExpr::BinaryOp(lhs, _, rhs) => {
            collect_calls_in_hir_expr(lhs, out);
            collect_calls_in_hir_expr(rhs, out);
        }
        HirExpr::Constructor { args, .. } => {
            for arg in args {
                collect_calls_in_hir_expr(arg, out);
            }
        }
        HirExpr::Record(fields) => {
            for (_, expr) in fields {
                collect_calls_in_hir_expr(expr, out);
            }
        }
        HirExpr::Array(items) => {
            for item in items {
                collect_calls_in_hir_expr(item, out);
            }
        }
        HirExpr::Index(arr, idx) => {
            collect_calls_in_hir_expr(arr, out);
            collect_calls_in_hir_expr(idx, out);
        }
        HirExpr::FieldAccess(expr, _) => collect_calls_in_hir_expr(expr, out),
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_calls_in_hir_expr(cond, out);
            collect_calls_in_hir_stmts(then_branch, out);
            if let Some(else_stmts) = else_branch {
                collect_calls_in_hir_stmts(else_stmts, out);
            }
        }
        HirExpr::Match { target, cases } => {
            collect_calls_in_hir_expr(target, out);
            for case in cases {
                collect_calls_in_hir_stmts(&case.body, out);
            }
        }
        HirExpr::Lambda { body, .. } => {
            collect_calls_in_hir_stmts(body, out);
        }
        HirExpr::Raise(expr) => collect_calls_in_hir_expr(expr, out),
        HirExpr::Handler { functions, .. } => {
            for f in functions {
                collect_calls_in_hir_stmts(&f.body, out);
            }
        }
        HirExpr::External(name, _, _) => out.push(name.clone()),
        HirExpr::Literal(_) | HirExpr::Variable(_, _) | HirExpr::Borrow(_, _) => {}
    }
}
