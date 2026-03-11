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
use crate::lang::parser;
use crate::lang::stdlib::{load_stdlib_nx_programs, resolve_import_path};
use std::collections::{HashMap, HashSet};
use std::fs;

#[derive(Debug)]
pub enum HirBuildError {
    ImportReadError {
        path: String,
        detail: String,
        span: Span,
    },
    ImportParseError {
        path: String,
        detail: String,
        span: Span,
    },
    CyclicImport {
        path: String,
        span: Span,
    },
    ImportItemNotFound {
        item: String,
        path: String,
        span: Span,
    },
}

impl std::fmt::Display for HirBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HirBuildError::ImportReadError { path, detail, .. } => {
                write!(f, "Failed to read '{}': {}", path, detail)
            }
            HirBuildError::ImportParseError { path, detail, .. } => {
                write!(f, "Failed to parse '{}': {}", path, detail)
            }
            HirBuildError::CyclicImport { path, .. } => {
                write!(f, "Cyclic import detected: {}", path)
            }
            HirBuildError::ImportItemNotFound { item, path, .. } => {
                write!(f, "Item '{}' not found in '{}'", item, path)
            }
        }
    }
}

impl HirBuildError {
    pub fn span(&self) -> &Span {
        match self {
            HirBuildError::ImportReadError { span, .. }
            | HirBuildError::ImportParseError { span, .. }
            | HirBuildError::CyclicImport { span, .. }
            | HirBuildError::ImportItemNotFound { span, .. } => span,
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

/// Cached info about an already-processed import module.
#[derive(Clone)]
struct ImportCache {
    /// Maps original definition name → registered HIR name
    name_map: HashMap<String, String>,
    /// Set of public definition names (for validation on reimport)
    public_names: HashSet<String>,
}

struct HirBuilder {
    ports: Vec<HirPort>,
    functions: Vec<HirFunction>,
    externals: Vec<HirExternal>,
    handler_bindings: HashMap<String, HirHandlerBinding>,
    import_stack: Vec<String>,
    enum_defs: Vec<EnumDef>,
    /// Tracks already-processed modules to avoid diamond import duplication
    imported_modules: HashMap<String, ImportCache>,
    /// Top-level `let` bindings with literal values, inlined at reference sites.
    global_constants: HashMap<String, Literal>,
}

impl HirBuilder {
    fn new() -> Self {
        HirBuilder {
            ports: Vec::new(),
            functions: Vec::new(),
            externals: Vec::new(),
            handler_bindings: HashMap::new(),
            import_stack: Vec::new(),
            enum_defs: Vec::new(),
            imported_modules: HashMap::new(),
            global_constants: HashMap::new(),
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
                    b.functions.iter().any(|f| reachable.contains(&f.name))
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
            enum_defs: self.enum_defs.clone(),
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
                    // Port names may contain dots (e.g. "stdio.Console" from aliased imports),
                    // so match known port names as prefixes rather than splitting on first dot
                    let mut resolved_as_port = false;
                    if called.contains('.') {
                        for (binding_name, binding) in &self.handler_bindings {
                            if let Some(method) = called
                                .strip_prefix(binding.port_name.as_str())
                                .and_then(|s| s.strip_prefix('.'))
                            {
                                let handler_fn = format!("__handler_{}_{}", binding_name, method);
                                if !reachable.contains(&handler_fn) {
                                    worklist.push(handler_fn);
                                }
                                resolved_as_port = true;
                            }
                        }
                    }
                    if !resolved_as_port && !reachable.contains(&called) {
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
        // Seed with builtin enum defs (List, Exn)
        use crate::lang::typecheck::{exn_enum_def, list_enum_def};
        self.enum_defs.push(list_enum_def());
        self.enum_defs.push(exn_enum_def());

        if let Ok(stdlib_programs) = load_stdlib_nx_programs() {
            for (_, stdlib_program) in stdlib_programs {
                let mut current_wasm_module: Option<String> = None;
                for def in &stdlib_program.definitions {
                    match &def.node {
                        TopLevel::Import(import) if import.is_external => {
                            current_wasm_module = Some(resolve_import_path(&import.path));
                        }
                        TopLevel::Enum(ed) => {
                            self.enum_defs.push(ed.clone());
                        }
                        TopLevel::Exception(_) => {}
                        TopLevel::Let(gl) if gl.is_public => {
                            if let Expr::External(wasm_name, _type_params, typ) = &gl.value.node {
                                if let Type::Arrow(params, ret, _requires, throws) = typ {
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
                                            throws: *throws.clone(),
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
                    current_wasm_module = Some(resolve_import_path(&import.path));
                }
                TopLevel::Import(import) => {
                    self.process_import(import, &def.span)?;
                }
                TopLevel::TypeDef(_) => {}
                TopLevel::Enum(ed) => {
                    self.enum_defs.push(ed.clone());
                }
                TopLevel::Exception(_) => {}
                TopLevel::Port(port) => {
                    self.ports.push(HirPort {
                        name: self.rename(&port.name, rename_map),
                        functions: port
                            .functions
                            .iter()
                            .map(|f| HirPortMethod {
                                name: f.name.clone(),
                            })
                            .collect(),
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
                            throws: _,
                            body,
                        } => {
                            // Desugar main(args: [string]) -> unit
                            // into main() -> unit with `let args = argv()` prepended
                            if name == "main"
                                && params.len() == 1
                                && params[0].typ
                                    == Type::List(Box::new(Type::String))
                            {
                                // Auto-import stdlib/proc.nx for argv
                                let proc_import = Import {
                                    path: "stdlib/proc.nx".to_string(),
                                    alias: None,
                                    items: vec!["argv".to_string()],
                                    is_external: false,
                                };
                                self.process_import(&proc_import, &def.span)?;

                                let arg_name = params[0].name.clone();
                                let mut desugared_body = vec![HirStmt::Let {
                                    name: arg_name,
                                    typ: Some(Type::List(Box::new(Type::String))),
                                    value: HirExpr::Call {
                                        func: "argv".to_string(),
                                        args: vec![],
                                    },
                                }];
                                desugared_body
                                    .extend(self.convert_stmts(body, rename_map));

                                self.functions.push(HirFunction {
                                    name: name.clone(),
                                    params: vec![],
                                    ret_type: ret_type.clone(),
                                    body: desugared_body,
                                    span: def.span.clone(),
                                });
                            } else {
                                self.functions.push(HirFunction {
                                    name: name.clone(),
                                    params: params
                                        .iter()
                                        .map(|p| HirParam {
                                            name: p.name.clone(),
                                            label: p.name.clone(),
                                            typ: p.typ.clone(),
                                        })
                                        .collect(),
                                    ret_type: ret_type.clone(),
                                    body: self.convert_stmts(body, rename_map),
                                    span: def.span.clone(),
                                });
                            }
                        }
                        Expr::External(wasm_name, _type_params, typ) => {
                            if let Type::Arrow(params, ret, _requires, throws) = typ {
                                if let Some(ref wasm_mod) = current_wasm_module {
                                    // If already defined, replace it (explicit import overrides auto-loaded)
                                    self.externals.retain(|e| e.name != name);
                                    self.externals.push(HirExternal {
                                        name: name.clone(),
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
                                        throws: *throws.clone(),
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
                                    params: hf
                                        .params
                                        .iter()
                                        .map(|p| HirParam {
                                            name: p.name.clone(),
                                            label: p.name.clone(),
                                            typ: p.typ.clone(),
                                        })
                                        .collect(),
                                    ret_type: hf.ret_type.clone(),
                                    body: self.convert_stmts(&hf.body, rename_map),
                                    span: def.span.clone(),
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
                        Expr::Literal(lit) => {
                            // Inline top-level literal constants at reference sites
                            self.global_constants.insert(name.clone(), lit.clone());
                        }
                        _ => {
                            // Other global let values not represented in HIR
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn process_import(&mut self, import: &Import, import_span: &Span) -> Result<(), HirBuildError> {
        let resolved_path = resolve_import_path(&import.path);
        if self.import_stack.iter().any(|p| p == &resolved_path) {
            return Err(HirBuildError::CyclicImport {
                path: resolved_path,
                span: import_span.clone(),
            });
        }

        // Diamond import dedup: if this module was already processed, reuse cached results
        if let Some(cache) = self.imported_modules.get(&resolved_path).cloned() {
            return self.handle_reimport(import, import_span, &cache);
        }

        self.import_stack.push(resolved_path.clone());
        let src =
            fs::read_to_string(&resolved_path).map_err(|e| HirBuildError::ImportReadError {
                path: resolved_path.clone(),
                detail: e.to_string(),
                span: import_span.clone(),
            })?;
        let imported_program =
            parser::parser()
                .parse(&src)
                .map_err(|e| HirBuildError::ImportParseError {
                    path: import.path.clone(),
                    detail: format!("{:?}", e),
                    span: import_span.clone(),
                })?;

        let rename_map = self.build_rename_map(&imported_program, import, import_span)?;
        let result = self.process_program(&imported_program, &rename_map);

        // Cache the processed module for future reimports
        let public_names: HashSet<String> = imported_program
            .definitions
            .iter()
            .filter_map(|def| match &def.node {
                TopLevel::Let(gl) if gl.is_public => Some(gl.name.clone()),
                TopLevel::Port(port) if port.is_public => Some(port.name.clone()),
                TopLevel::Enum(ed) if ed.is_public => Some(ed.name.clone()),
                TopLevel::TypeDef(td) if td.is_public => Some(td.name.clone()),
                _ => None,
            })
            .collect();
        self.imported_modules.insert(
            resolved_path.clone(),
            ImportCache {
                name_map: rename_map,
                public_names,
            },
        );

        self.import_stack.pop();
        result
    }

    /// Handle reimport of an already-processed module.
    /// Creates aliases for newly-selected items that were registered with a prefix.
    fn handle_reimport(
        &mut self,
        import: &Import,
        import_span: &Span,
        cache: &ImportCache,
    ) -> Result<(), HirBuildError> {
        if import.items.is_empty() {
            // Wildcard reimport: create aliases with the requested prefix
            let alias = import
                .alias
                .clone()
                .unwrap_or_else(|| get_default_alias(&import.path));
            for (original_name, registered_name) in &cache.name_map {
                let aliased_name = format!("{}.{}", alias, original_name);
                if aliased_name != *registered_name {
                    self.create_function_alias(&aliased_name, registered_name);
                    self.create_external_alias(&aliased_name, registered_name);
                }
            }
            return Ok(());
        }

        // Selective reimport: validate and create aliases for items registered with a prefix
        for item in &import.items {
            if !cache.public_names.contains(item) {
                return Err(HirBuildError::ImportItemNotFound {
                    item: item.clone(),
                    path: import.path.clone(),
                    span: import_span.clone(),
                });
            }
            if let Some(registered_name) = cache.name_map.get(item) {
                if registered_name != item {
                    // Item was registered with a prefix — create an alias with the bare name
                    self.create_function_alias(item, registered_name);
                    self.create_external_alias(item, registered_name);
                }
            }
        }
        Ok(())
    }

    /// Clone a function under a new name (alias), replacing any existing function with that name.
    fn create_function_alias(&mut self, alias_name: &str, registered_name: &str) {
        if let Some(func) = self
            .functions
            .iter()
            .find(|f| f.name == registered_name)
            .cloned()
        {
            self.functions.retain(|f| f.name != alias_name);
            let mut aliased = func;
            aliased.name = alias_name.to_string();
            self.functions.push(aliased);
        }
    }

    /// Clone an external under a new name (alias), replacing any existing external with that name.
    fn create_external_alias(&mut self, alias_name: &str, registered_name: &str) {
        if let Some(ext) = self
            .externals
            .iter()
            .find(|e| e.name == registered_name)
            .cloned()
        {
            // Remove any existing external with the alias name (e.g. stdlib auto-loaded version)
            self.externals.retain(|e| e.name != alias_name);
            let mut aliased = ext;
            aliased.name = alias_name.to_string();
            self.externals.push(aliased);
        }
    }

    fn build_rename_map(
        &self,
        program: &Program,
        import: &Import,
        import_span: &Span,
    ) -> Result<HashMap<String, String>, HirBuildError> {
        let mut map = HashMap::new();

        if import.items.is_empty() {
            // Wildcard import: prefix everything with alias
            let alias = import
                .alias
                .clone()
                .unwrap_or_else(|| get_default_alias(&import.path));
            for def in &program.definitions {
                match &def.node {
                    TopLevel::Let(gl) => {
                        map.insert(gl.name.clone(), format!("{}.{}", alias, gl.name));
                    }
                    TopLevel::Port(port) => {
                        map.insert(port.name.clone(), format!("{}.{}", alias, port.name));
                    }
                    _ => {}
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
                    span: import_span.clone(),
                });
            }
        }

        if let Some(alias) = &import.alias {
            for def in &program.definitions {
                match &def.node {
                    TopLevel::Let(gl) => {
                        if gl.is_public && selected.contains(&gl.name) {
                            map.insert(gl.name.clone(), gl.name.clone());
                        } else {
                            map.insert(gl.name.clone(), format!("{}.{}", alias, gl.name));
                        }
                    }
                    TopLevel::Port(port) => {
                        if port.is_public && selected.contains(&port.name) {
                            map.insert(port.name.clone(), port.name.clone());
                        } else {
                            map.insert(port.name.clone(), format!("{}.{}", alias, port.name));
                        }
                    }
                    _ => {}
                }
            }
        } else {
            let alias_prefix = {
                let path_hash = {
                    let mut h: u64 = 0xcbf29ce484222325;
                    for b in import.path.bytes() {
                        h ^= b as u64;
                        h = h.wrapping_mul(0x100000001b3);
                    }
                    h % 10000
                };
                format!("__import_{}_{}", self.import_stack.len(), path_hash)
            };
            for def in &program.definitions {
                match &def.node {
                    TopLevel::Let(gl) => {
                        if selected.contains(&gl.name) {
                            map.insert(gl.name.clone(), gl.name.clone());
                        } else {
                            map.insert(gl.name.clone(), format!("{}_{}", alias_prefix, gl.name));
                        }
                    }
                    TopLevel::Port(port) => {
                        if selected.contains(&port.name) {
                            map.insert(port.name.clone(), port.name.clone());
                        } else {
                            map.insert(
                                port.name.clone(),
                                format!("{}_{}", alias_prefix, port.name),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(map)
    }

    fn rename(&self, name: &str, map: &HashMap<String, String>) -> String {
        map.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    // ---- AST → HIR conversion helpers ----

    fn convert_stmts(
        &self,
        stmts: &[Spanned<Stmt>],
        rename_map: &HashMap<String, String>,
    ) -> Vec<HirStmt> {
        stmts
            .iter()
            .filter_map(|s| self.convert_stmt(&s.node, rename_map))
            .collect()
    }

    fn convert_stmt(&self, stmt: &Stmt, rename_map: &HashMap<String, String>) -> Option<HirStmt> {
        match stmt {
            Stmt::Let {
                name, typ, value, ..
            } => Some(HirStmt::Let {
                name: name.clone(),
                typ: typ.clone(),
                value: self.convert_expr(value, rename_map),
            }),
            Stmt::Expr(expr) => Some(HirStmt::Expr(self.convert_expr(expr, rename_map))),
            Stmt::Return(expr) => Some(HirStmt::Return(self.convert_expr(expr, rename_map))),
            Stmt::Assign { target, value } => Some(HirStmt::Assign {
                target: self.convert_expr(target, rename_map),
                value: self.convert_expr(value, rename_map),
            }),
            Stmt::Conc(tasks) => {
                let hir_tasks: Vec<HirFunction> = tasks
                    .iter()
                    .map(|t| HirFunction {
                        name: t.name.clone(),
                        params: vec![],
                        ret_type: Type::Unit,
                        body: self.convert_stmts(&t.body, rename_map),
                        span: 0..0, // synthesized task function
                    })
                    .collect();
                Some(HirStmt::Conc(hir_tasks))
            }
            Stmt::Try {
                body,
                catch_param,
                catch_body,
            } => Some(HirStmt::Try {
                body: self.convert_stmts(body, rename_map),
                catch_param: catch_param.clone(),
                catch_body: self.convert_stmts(catch_body, rename_map),
            }),
            Stmt::Inject { handlers, body } => Some(HirStmt::Inject {
                handlers: handlers
                    .iter()
                    .map(|h| self.rename(h, rename_map))
                    .collect(),
                body: self.convert_stmts(body, rename_map),
            }),
        }
    }

    fn convert_expr(&self, expr: &Spanned<Expr>, rename_map: &HashMap<String, String>) -> HirExpr {
        match &expr.node {
            Expr::Literal(lit) => HirExpr::Literal(lit.clone()),
            Expr::Variable(name, sigil) => {
                let resolved = self.rename(name, rename_map);
                if let Some(lit) = self.global_constants.get(&resolved) {
                    HirExpr::Literal(lit.clone())
                } else {
                    HirExpr::Variable(resolved, sigil.clone())
                }
            }
            Expr::BinaryOp(lhs, op, rhs) => HirExpr::BinaryOp(
                Box::new(self.convert_expr(lhs, rename_map)),
                *op,
                Box::new(self.convert_expr(rhs, rename_map)),
            ),
            Expr::Borrow(name, sigil) => {
                HirExpr::Borrow(self.rename(name, rename_map), sigil.clone())
            }
            Expr::Call { func, args } => {
                let renamed_func = self.rename(func, rename_map);
                HirExpr::Call {
                    func: renamed_func,
                    args: args
                        .iter()
                        .map(|(n, e)| (n.clone(), self.convert_expr(e, rename_map)))
                        .collect(),
                }
            }
            Expr::Constructor(name, args) => {
                // Zero-arg constructors that match a global constant are inlined
                if args.is_empty() {
                    let resolved = self.rename(name, rename_map);
                    if let Some(lit) = self.global_constants.get(&resolved) {
                        return HirExpr::Literal(lit.clone());
                    }
                }
                HirExpr::Constructor {
                    variant: name.clone(),
                    args: args
                        .iter()
                        .map(|(_, e)| self.convert_expr(e, rename_map))
                        .collect(),
                }
            }
            Expr::Record(fields) => HirExpr::Record(
                fields
                    .iter()
                    .map(|(n, e)| (n.clone(), self.convert_expr(e, rename_map)))
                    .collect(),
            ),
            Expr::Array(items) => HirExpr::Array(
                items
                    .iter()
                    .map(|e| self.convert_expr(e, rename_map))
                    .collect(),
            ),
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
            Expr::Index(arr, idx) => HirExpr::Index(
                Box::new(self.convert_expr(arr, rename_map)),
                Box::new(self.convert_expr(idx, rename_map)),
            ),
            Expr::FieldAccess(expr, field) => {
                HirExpr::FieldAccess(Box::new(self.convert_expr(expr, rename_map)), field.clone())
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => HirExpr::If {
                cond: Box::new(self.convert_expr(cond, rename_map)),
                then_branch: self.convert_stmts(then_branch, rename_map),
                else_branch: else_branch
                    .as_ref()
                    .map(|b| self.convert_stmts(b, rename_map)),
            },
            Expr::Match { target, cases } => HirExpr::Match {
                target: Box::new(self.convert_expr(target, rename_map)),
                cases: cases
                    .iter()
                    .map(|c| HirMatchCase {
                        pattern: self.convert_pattern(&c.pattern.node),
                        body: self.convert_stmts(&c.body, rename_map),
                    })
                    .collect(),
            },
            Expr::While { cond, body } => HirExpr::While {
                cond: Box::new(self.convert_expr(cond, rename_map)),
                body: self.convert_stmts(body, rename_map),
            },
            Expr::For {
                var,
                start,
                end_expr,
                body,
            } => HirExpr::For {
                var: var.clone(),
                start: Box::new(self.convert_expr(start, rename_map)),
                end_expr: Box::new(self.convert_expr(end_expr, rename_map)),
                body: self.convert_stmts(body, rename_map),
            },
            Expr::Lambda {
                type_params: _,
                params,
                ret_type,
                requires: _,
                throws: _,
                body,
            } => HirExpr::Lambda {
                params: params
                    .iter()
                    .map(|p| HirParam {
                        name: p.name.clone(),
                        label: p.name.clone(),
                        typ: p.typ.clone(),
                    })
                    .collect(),
                ret_type: ret_type.clone(),
                body: self.convert_stmts(body, rename_map),
            },
            Expr::Raise(expr) => HirExpr::Raise(Box::new(self.convert_expr(expr, rename_map))),
            Expr::External(sym, tparams, typ) => {
                HirExpr::External(sym.clone(), tparams.clone(), typ.clone())
            }
            Expr::Handler {
                coeffect_name: _,
                requires: _,
                functions,
            } => HirExpr::Handler {
                functions: functions
                    .iter()
                    .map(|f| HirFunction {
                        name: f.name.clone(),
                        params: f
                            .params
                            .iter()
                            .map(|p| HirParam {
                                name: p.name.clone(),
                                label: p.name.clone(),
                                typ: p.typ.clone(),
                            })
                            .collect(),
                        ret_type: f.ret_type.clone(),
                        body: self.convert_stmts(&f.body, rename_map),
                        span: 0..0, // inline handler function
                    })
                    .collect(),
            },
        }
    }

    fn convert_pattern(&self, pattern: &Pattern) -> HirPattern {
        match pattern {
            Pattern::Literal(lit) => HirPattern::Literal(lit.clone()),
            Pattern::Variable(name, sigil) => HirPattern::Variable(name.clone(), sigil.clone()),
            Pattern::Constructor(name, fields) => HirPattern::Constructor {
                variant: name.clone(),
                fields: fields
                    .iter()
                    .map(|(label, p)| (label.clone(), self.convert_pattern(&p.node)))
                    .collect(),
            },
            Pattern::Record(fields, open) => HirPattern::Record(
                fields
                    .iter()
                    .map(|(n, p)| (n.clone(), self.convert_pattern(&p.node)))
                    .collect(),
                *open,
            ),
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
                body, catch_body, ..
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
        HirExpr::While { cond, body } => {
            collect_calls_in_hir_expr(cond, out);
            collect_calls_in_hir_stmts(body, out);
        }
        HirExpr::For {
            start,
            end_expr,
            body,
            ..
        } => {
            collect_calls_in_hir_expr(start, out);
            collect_calls_in_hir_expr(end_expr, out);
            collect_calls_in_hir_stmts(body, out);
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
