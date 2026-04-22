//! AST → MIR (module flatten, name resolve, handler collect, port resolve)
//!
//! Single-pass transformation from parsed AST to MIR:
//! - Module resolution (imports, file loading, cycle detection)
//! - Name resolution (qualify identifiers, resolve constructors)
//! - Handler collection (synthesize handler functions)
//! - Port call resolution (static dispatch to handler functions)
//! - For-loop desugaring (→ while loop)
//! - Inject inlining (scope-based handler activation)

use crate::constants::handler_func_name;
use crate::intern::Symbol;
use crate::ir::mir::*;
use crate::lang::ast::*;
use crate::lang::parser;
use crate::lang::stdlib::{load_stdlib_nx_programs, resolve_import_path};
use std::collections::{HashMap, HashSet};
use std::path::Path;

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
    UnresolvedPort {
        port: String,
        method: String,
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
            HirBuildError::UnresolvedPort { port, method, .. } => {
                write!(f, "Unresolved port method: {}.{}", port, method)
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
            | HirBuildError::ImportItemNotFound { span, .. }
            | HirBuildError::UnresolvedPort { span, .. } => span,
        }
    }
}

/// Builds a MirProgram directly from a parsed AST Program.
/// Performs module resolution, name resolution, handler collection,
/// port call resolution, and for-loop desugaring in a single pass.
#[tracing::instrument(skip_all, name = "build_hir")]
pub fn build_hir(program: &Program) -> Result<MirProgram, HirBuildError> {
    let mut builder = MirBuilder::new();
    builder.build(program)
}

/// Sanitize a source path to a wasm-safe identifier segment.
fn sanitize_path(path: &str) -> String {
    path.replace(['/', '.'], "_")
}

/// Canonical global name: "sanitized_source_path#name".
fn canonical_name(source_path: &str, name: &str) -> String {
    format!("{}#{}", sanitize_path(source_path), name)
}

/// Importer's view entries for an import. For wildcard `import * as a from "A"`,
/// produces `a.foo -> canonical(A, foo)`. For selective `import { foo as bar }`,
/// produces `bar -> canonical(A, foo)`.
fn compute_importer_entries(
    import: &Import,
    program: &Program,
    source_path: &str,
) -> HashMap<String, String> {
    let mut entries = HashMap::new();
    let canonical = |name: &str| canonical_name(source_path, name);

    if import.items.is_empty() {
        let alias = import
            .alias
            .clone()
            .unwrap_or_else(|| get_default_alias(&import.path));
        for def in &program.definitions {
            match &def.node {
                TopLevel::Let(gl) => {
                    entries.insert(format!("{}.{}", alias, gl.name), canonical(&gl.name));
                }
                TopLevel::Port(port) => {
                    entries.insert(format!("{}.{}", alias, port.name), canonical(&port.name));
                }
                _ => {}
            }
        }
        return entries;
    }

    // Selective import
    for item in &import.items {
        let visible = item.alias.as_ref().unwrap_or(&item.name).clone();
        for def in &program.definitions {
            match &def.node {
                TopLevel::Let(gl) if gl.is_public && gl.name == item.name => {
                    entries.insert(visible.clone(), canonical(&gl.name));
                }
                TopLevel::Port(port) if port.is_public && port.name == item.name => {
                    entries.insert(visible.clone(), canonical(&port.name));
                }
                _ => {}
            }
        }
    }
    entries
}

/// Convert a byte offset to a 1-based line number.
fn offset_to_line(source: &str, offset: usize) -> u32 {
    let end = offset.min(source.len());
    source.as_bytes()[..end]
        .iter()
        .filter(|&&b| b == b'\n')
        .count() as u32
        + 1
}

/// Cached info about an already-processed import module.
#[derive(Clone)]
struct ImportCache {
    /// Maps original definition name → registered name
    name_map: HashMap<String, String>,
    /// Set of public definition names (for validation on reimport)
    public_names: HashSet<String>,
}

/// Tracks which handler is active for each port in the current scope
#[derive(Clone)]
struct HandlerScope {
    /// port_name → handler_binding_name
    active: HashMap<String, String>,
}

/// Info about a handler binding (collected during build phase)
struct HandlerBinding {
    port_name: String,
}

/// A pending function whose body needs to be converted in the second pass
#[derive(Clone)]
struct PendingFunction {
    name: String,
    params: Vec<MirParam>,
    ret_type: Type,
    body: Vec<Spanned<Stmt>>,
    rename_map: HashMap<String, String>,
    span: Span,
    /// MIR statements prepended before the converted body (e.g., main(args) desugaring)
    preamble: Vec<MirStmt>,
    source_file: Option<String>,
    source_line: Option<u32>,
    /// Captured variable names (for closure-converted lambdas)
    captures: Vec<Symbol>,
}

struct MirBuilder {
    /// port_name → ordered method names
    port_methods: HashMap<String, Vec<String>>,
    functions: Vec<MirFunction>,
    externals: Vec<MirExternal>,
    /// binding_name → handler info
    handler_bindings: HashMap<String, HandlerBinding>,
    /// binding_name → port_name
    handler_port_names: HashMap<String, String>,
    /// Return types for all known functions (populated during pass 1)
    fn_ret_types: HashMap<String, Type>,
    import_stack: Vec<String>,
    enum_defs: Vec<EnumDef>,
    /// Tracks already-processed modules to avoid diamond import duplication
    imported_modules: HashMap<String, ImportCache>,
    /// Top-level `let` bindings with literal values, inlined at reference sites.
    global_constants: HashMap<String, Literal>,
    /// Functions collected during declaration pass, bodies converted in second pass
    pending_functions: Vec<PendingFunction>,
    /// Current source file path (set during collect_declarations)
    current_source_file: Option<String>,
    /// Current source text (for byte-offset → line-number conversion)
    current_source_text: Option<String>,
    /// Counter for generating unique lambda names
    lambda_counter: usize,
    /// Full Arrow types for known functions (for FuncRef type resolution)
    fn_types: HashMap<String, Type>,
    /// Types of local variables in the current conversion scope (for closure capture type resolution)
    scope_var_types: HashMap<String, Type>,
    /// Exception group name → member exception names (for catch expansion)
    exception_groups: HashMap<String, Vec<String>>,
}

impl MirBuilder {
    fn source_line_at(&self, span: &Span) -> Option<u32> {
        self.current_source_text
            .as_ref()
            .map(|src| offset_to_line(src, span.start))
    }

    fn new() -> Self {
        MirBuilder {
            port_methods: HashMap::new(),
            functions: Vec::new(),
            externals: Vec::new(),
            handler_bindings: HashMap::new(),
            handler_port_names: HashMap::new(),
            fn_ret_types: HashMap::new(),
            import_stack: Vec::new(),
            enum_defs: Vec::new(),
            imported_modules: HashMap::new(),
            global_constants: HashMap::new(),
            pending_functions: Vec::new(),
            current_source_file: None,
            current_source_text: None,
            lambda_counter: 0,
            fn_types: HashMap::new(),
            scope_var_types: HashMap::new(),
            exception_groups: HashMap::new(),
        }
    }

    fn build(&mut self, program: &Program) -> Result<MirProgram, HirBuildError> {
        // Load stdlib definitions (enums, exceptions, externals)
        self.load_stdlib()?;

        // Set source context for the entry file
        self.current_source_file = program.source_file.clone();
        self.current_source_text = program.source_text.clone();

        // Pass 1: Collect all declarations (ports, handlers, externals, etc.)
        // Function bodies are stored as pending for the second pass.
        let mut top_rename_map = HashMap::new();
        self.collect_declarations(program, &mut top_rename_map)?;

        // Pass 2: Convert all pending function bodies with full handler scope.
        // Loop until no new lambda-lifted functions are generated.
        let scope = self.build_global_scope();
        loop {
            let pending = std::mem::take(&mut self.pending_functions);
            if pending.is_empty() {
                break;
            }
            for pf in pending {
                // Set up scope_var_types from function params for capture type resolution
                self.scope_var_types.clear();
                for p in &pf.params {
                    self.scope_var_types
                        .insert(p.name.to_string(), p.typ.clone());
                }
                let mut body = pf.preamble;
                body.extend(self.convert_stmts(&pf.body, &pf.rename_map, &scope)?);
                // Implicit return: if the last statement is a bare expression
                // (not already a Return), promote it to Return so the value
                // is used as the function's return value.
                if let Some(MirStmt::Expr(expr)) = body.last().cloned() {
                    if !matches!(
                        expr,
                        MirExpr::If { .. } | MirExpr::Match { .. } | MirExpr::While { .. }
                    ) {
                        *body.last_mut().unwrap() = MirStmt::Return(expr);
                    }
                }
                self.functions.push(MirFunction {
                    name: Symbol::from(&pf.name),
                    params: pf.params,
                    ret_type: pf.ret_type,
                    body,
                    span: pf.span,
                    source_file: pf.source_file,
                    source_line: pf.source_line,
                    captures: pf.captures.clone(),
                });
            }
        }

        // Collect reachable functions from main
        let reachable = self.collect_reachable();

        // Filter to only reachable functions and externals.
        // Drain instead of clone — builder is not used after build() returns.
        let functions: Vec<MirFunction> = self
            .functions
            .drain(..)
            .filter(|f| reachable.contains(&f.name))
            .collect();
        let externals: Vec<MirExternal> = self
            .externals
            .drain(..)
            .filter(|e| reachable.contains(&e.name))
            .collect();

        Ok(MirProgram {
            functions,
            externals,
            enum_defs: std::mem::take(&mut self.enum_defs),
        })
    }

    /// Build the global handler scope: for each port, the last registered handler wins
    fn build_global_scope(&self) -> HandlerScope {
        let mut active = HashMap::new();
        for (binding_name, binding) in &self.handler_bindings {
            active.insert(binding.port_name.clone(), binding_name.clone());
        }
        HandlerScope { active }
    }

    /// Collect all function names transitively reachable from "main"
    fn collect_reachable(&self) -> HashSet<Symbol> {
        let mut reachable = HashSet::new();
        let mut worklist: Vec<Symbol> = vec![Symbol::from("main")];

        while let Some(name) = worklist.pop() {
            if !reachable.insert(name) {
                continue;
            }
            // Find function body and collect calls
            if let Some(func) = self.functions.iter().find(|f| f.name == name) {
                let mut calls = Vec::new();
                collect_calls_in_mir_stmts(&func.body, &mut calls);
                for called in calls {
                    if !reachable.contains(&called) {
                        worklist.push(called);
                    }
                }
            }
        }
        reachable
    }

    /// Look up the return type of a function by name
    fn lookup_ret_type(&self, func_name: &str) -> Type {
        if let Some(ty) = self.fn_ret_types.get(func_name) {
            return ty.clone();
        }
        for ext in &self.externals {
            if ext.name == func_name {
                return ext.ret_type.clone();
            }
        }
        Type::I64
    }

    /// Register an exception definition as a variant in the Exn enum within enum_defs.
    /// This ensures that the LIR lowerer can look up field labels for exception constructors,
    /// which is needed for correct sorted field layout in pattern matching.
    fn register_exception_in_enum_defs(&mut self, ex: &ExceptionDef) {
        let variant = VariantDef {
            name: ex.name.clone(),
            fields: ex.fields.clone(),
        };
        // Find the Exn enum and add this variant (if not already present)
        if let Some(exn_def) = self.enum_defs.iter_mut().find(|d| d.name == "Exn") {
            if !exn_def.variants.iter().any(|v| v.name == ex.name) {
                exn_def.variants.push(variant);
            }
        }
    }

    fn load_stdlib(&mut self) -> Result<(), HirBuildError> {
        // Seed with builtin enum defs (List, Exn)
        use crate::lang::typecheck::{exn_enum_def, list_enum_def};
        self.enum_defs.push(list_enum_def());
        self.enum_defs.push(exn_enum_def());

        if let Ok(stdlib_programs) = load_stdlib_nx_programs() {
            for (path, stdlib_program) in stdlib_programs {
                let wit_interface = stdlib_nx_to_wit_interface(&path);
                let mut current_wasm_module: Option<String> = None;
                for def in &stdlib_program.definitions {
                    match &def.node {
                        TopLevel::Import(import) if import.is_external => {
                            let resolved = resolve_import_path(&import.path);
                            current_wasm_module = Some(resolve_stdlib_wit_module(
                                &resolved,
                                wit_interface.as_deref(),
                            ));
                        }
                        TopLevel::Enum(ed) => {
                            self.enum_defs.push(ed.clone());
                        }
                        TopLevel::Exception(ex) => {
                            self.register_exception_in_enum_defs(ex);
                        }
                        TopLevel::Let(gl) if gl.is_public => {
                            if let Expr::External(wasm_name, _type_params, typ) = &gl.value.node {
                                if let Type::Arrow(params, ret, _requires, throws) = typ {
                                    if let Some(ref wasm_mod) = current_wasm_module {
                                        // Skip if already loaded (avoid duplicates)
                                        if self.externals.iter().any(|e| e.name == gl.name) {
                                            continue;
                                        }
                                        // Use WIT-canonical kebab-case names for component-model modules.
                                        let effective_wasm_name = if wasm_mod.contains(':') {
                                            wit_canonical_name(wasm_name)
                                        } else {
                                            wasm_name.to_string()
                                        };
                                        self.externals.push(MirExternal {
                                            name: Symbol::from(&gl.name),
                                            wasm_module: Symbol::from(wasm_mod.as_str()),
                                            wasm_name: Symbol::from(effective_wasm_name.as_str()),
                                            params: params
                                                .iter()
                                                .map(|(n, t)| MirParam {
                                                    name: Symbol::from(n.as_str()),
                                                    label: Symbol::from(n.as_str()),
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

    /// Pass 1: Collect all declarations without converting function bodies.
    /// Lambda/handler bodies are stored as PendingFunction for pass 2.
    fn collect_declarations(
        &mut self,
        program: &Program,
        rename_map: &mut HashMap<String, String>,
    ) -> Result<(), HirBuildError> {
        let mut current_wasm_module: Option<String> = None;
        let wit_interface = self
            .current_source_file
            .as_ref()
            .and_then(|p| stdlib_nx_to_wit_interface(Path::new(p)));

        // Pre-pass: canonicalize the current file's own top-level definitions.
        // `main` in the entry file keeps its bare name so the wasm `main` export
        // is emitted without renaming.
        let current_src = self
            .current_source_file
            .clone()
            .unwrap_or_else(|| "__entry__".to_string());
        let is_entry = self.import_stack.is_empty();
        for def in &program.definitions {
            match &def.node {
                TopLevel::Let(gl) => {
                    if is_entry && gl.name == "main" {
                        rename_map.insert(gl.name.clone(), "main".to_string());
                    } else {
                        rename_map
                            .insert(gl.name.clone(), canonical_name(&current_src, &gl.name));
                    }
                }
                TopLevel::Port(port) => {
                    rename_map
                        .insert(port.name.clone(), canonical_name(&current_src, &port.name));
                }
                _ => {}
            }
        }

        // Pre-pass B: populate rename_map from all non-external imports before
        // the main loop captures function bodies into pending_functions. Imports
        // may legally appear anywhere in the source (e.g. nxc/typecheck/check.nx
        // uses `str_length` at line 257 and imports it at line 448) but pending
        // function bodies must resolve names regardless of source ordering.
        // External imports stay in the main loop because they set
        // current_wasm_module for subsequent external declarations.
        for def in &program.definitions {
            if let TopLevel::Import(import) = &def.node {
                if !import.is_external {
                    self.process_import(import, &def.span, rename_map)?;
                }
            }
        }

        for def in &program.definitions {
            match &def.node {
                TopLevel::Import(import) if import.is_external => {
                    let resolved = resolve_import_path(&import.path);
                    current_wasm_module = Some(resolve_stdlib_wit_module(
                        &resolved,
                        wit_interface.as_deref(),
                    ));
                }
                TopLevel::Import(_) => {}  // non-external processed in pre-pass B
                TopLevel::TypeDef(_) => {}
                TopLevel::Enum(ed) => {
                    self.enum_defs.push(ed.clone());
                }
                TopLevel::Exception(ex) => {
                    self.register_exception_in_enum_defs(ex);
                }
                TopLevel::ExceptionGroup(eg) => {
                    self.exception_groups
                        .insert(eg.name.clone(), eg.members.clone());
                }
                TopLevel::Port(port) => {
                    let port_name = self.rename(&port.name, rename_map);
                    let methods: Vec<String> =
                        port.functions.iter().map(|f| f.name.clone()).collect();
                    self.port_methods.insert(port_name, methods);
                }
                TopLevel::Let(gl) => {
                    let name = self.rename(&gl.name, rename_map);
                    match &gl.value.node {
                        Expr::Lambda {
                            type_params: _,
                            params,
                            ret_type,
                            requires,
                            throws,
                            body,
                        } => {
                            // Desugar main(args: [string]) -> unit
                            if name == "main"
                                && params.len() == 1
                                && params[0].typ == Type::List(Box::new(Type::String))
                            {
                                let proc_import = Import {
                                    path: "stdlib/proc.nx".to_string(),
                                    alias: None,
                                    items: vec![ImportItem {
                                        name: "argv".to_string(),
                                        alias: None,
                                    }],
                                    is_external: false,
                                };
                                self.process_import(&proc_import, &def.span, rename_map)?;

                                let arg_name = params[0].name.clone();
                                let argv_canonical = self.rename("argv", rename_map);
                                let preamble = vec![MirStmt::Let {
                                    name: Symbol::from(&arg_name),
                                    typ: Type::List(Box::new(Type::String)),
                                    expr: MirExpr::Call {
                                        func: Symbol::from(argv_canonical),
                                        args: vec![],
                                        ret_type: Type::List(Box::new(Type::String)),
                                    },
                                }];
                                self.fn_ret_types.insert(name.clone(), ret_type.clone());
                                self.pending_functions.push(PendingFunction {
                                    name,
                                    params: vec![],
                                    ret_type: ret_type.clone(),
                                    body: body.clone(),
                                    rename_map: rename_map.clone(),
                                    source_file: self.current_source_file.clone(),
                                    source_line: self.source_line_at(&def.span),
                                    span: def.span.clone(),
                                    preamble,
                                    captures: vec![],
                                });
                            } else {
                                self.fn_ret_types.insert(name.clone(), ret_type.clone());
                                let arrow_type = Type::Arrow(
                                    params
                                        .iter()
                                        .map(|p| (p.name.clone(), p.typ.clone()))
                                        .collect(),
                                    Box::new(ret_type.clone()),
                                    Box::new(requires.clone()),
                                    Box::new(throws.clone()),
                                );
                                self.fn_types.insert(name.clone(), arrow_type);
                                self.pending_functions.push(PendingFunction {
                                    name,
                                    params: params
                                        .iter()
                                        .map(|p| MirParam {
                                            name: Symbol::from(&p.name),
                                            label: Symbol::from(&p.name),
                                            typ: p.typ.clone(),
                                        })
                                        .collect(),
                                    ret_type: ret_type.clone(),
                                    body: body.clone(),
                                    rename_map: rename_map.clone(),
                                    source_file: self.current_source_file.clone(),
                                    source_line: self.source_line_at(&def.span),
                                    span: def.span.clone(),
                                    preamble: vec![],
                                    captures: vec![],
                                });
                            }
                        }
                        Expr::External(wasm_name, _type_params, typ) => {
                            if let Type::Arrow(params, ret, _requires, throws) = typ {
                                let resolved_mod = current_wasm_module.clone().or_else(|| {
                                    self.externals
                                        .iter()
                                        .find(|e| e.wasm_name.as_ref() == wasm_name
                                            || wit_canonical_name(wasm_name) == e.wasm_name.as_ref())
                                        .map(|e| e.wasm_module.as_ref().to_string())
                                });
                                if let Some(wasm_mod) = resolved_mod {
                                    let name_sym = Symbol::from(&name);
                                    self.externals.retain(|e| e.name != name_sym);
                                    let effective_wasm_name = if wasm_mod.contains(':') {
                                        wit_canonical_name(wasm_name)
                                    } else {
                                        wasm_name.to_string()
                                    };
                                    self.externals.push(MirExternal {
                                        name: name_sym,
                                        wasm_module: Symbol::from(wasm_mod.as_str()),
                                        wasm_name: Symbol::from(effective_wasm_name.as_str()),
                                        params: params
                                            .iter()
                                            .map(|(n, t)| MirParam {
                                                name: Symbol::from(n.as_str()),
                                                label: Symbol::from(n.as_str()),
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
                            let port_name = self.rename(coeffect_name, rename_map);
                            for hf in handler_fns {
                                let synth_name = handler_func_name(&name, &hf.name);
                                self.fn_ret_types
                                    .insert(synth_name.clone(), hf.ret_type.clone());
                                self.pending_functions.push(PendingFunction {
                                    name: synth_name,
                                    params: hf
                                        .params
                                        .iter()
                                        .map(|p| MirParam {
                                            name: Symbol::from(&p.name),
                                            label: Symbol::from(&p.name),
                                            typ: p.typ.clone(),
                                        })
                                        .collect(),
                                    ret_type: hf.ret_type.clone(),
                                    body: hf.body.clone(),
                                    rename_map: rename_map.clone(),
                                    source_file: self.current_source_file.clone(),
                                    source_line: self.source_line_at(&def.span),
                                    span: def.span.clone(),
                                    preamble: vec![],
                                    captures: vec![],
                                });
                            }
                            self.handler_port_names
                                .insert(name.clone(), port_name.clone());
                            self.handler_bindings
                                .insert(name.clone(), HandlerBinding { port_name });
                        }
                        Expr::Literal(lit) => {
                            self.global_constants.insert(name.clone(), lit.clone());
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn process_import(
        &mut self,
        import: &Import,
        import_span: &Span,
        caller_rename_map: &mut HashMap<String, String>,
    ) -> Result<(), HirBuildError> {
        let resolved_path = resolve_import_path(&import.path);
        if self.import_stack.iter().any(|p| p == &resolved_path) {
            return Err(HirBuildError::CyclicImport {
                path: resolved_path,
                span: import_span.clone(),
            });
        }

        // Diamond import dedup
        if let Some(cache) = self.imported_modules.get(&resolved_path).cloned() {
            return self.handle_reimport(import, import_span, &cache, caller_rename_map);
        }

        self.import_stack.push(resolved_path.clone());
        let src = std::fs::read_to_string(&resolved_path).map_err(|e| {
            HirBuildError::ImportReadError {
                path: resolved_path.clone(),
                detail: e.to_string(),
                span: import_span.clone(),
            }
        })?;
        let imported_program =
            parser::parser()
                .parse(&src)
                .map_err(|e| HirBuildError::ImportParseError {
                    path: import.path.clone(),
                    detail: format!("{:?}", e),
                    span: import_span.clone(),
                })?;

        // Save and swap source context for the imported module
        let saved_source_file = self.current_source_file.take();
        let saved_source_text = self.current_source_text.take();
        self.current_source_file = Some(resolved_path.clone());
        self.current_source_text = Some(src.clone());

        let mut rename_map = self.build_rename_map(&imported_program, import, import_span)?;
        let result = self.collect_declarations(&imported_program, &mut rename_map);

        // Populate caller's scope with importer-view entries: the visible names
        // (alias.foo for wildcard, or foo/bar for selective imports) -> canonical.
        let importer_entries =
            compute_importer_entries(import, &imported_program, &resolved_path);
        for (k, v) in importer_entries {
            caller_rename_map.insert(k, v);
        }

        // Restore source context
        self.current_source_file = saved_source_file;
        self.current_source_text = saved_source_text;

        // Cache the processed module
        let public_names: HashSet<String> = imported_program
            .definitions
            .iter()
            .flat_map(|def| match &def.node {
                TopLevel::Let(gl) if gl.is_public => vec![gl.name.clone()],
                TopLevel::Port(port) if port.is_public => vec![port.name.clone()],
                TopLevel::Enum(ed) if ed.is_public => {
                    let mut names = vec![ed.name.clone()];
                    for v in &ed.variants {
                        names.push(v.name.clone());
                    }
                    names
                }
                TopLevel::TypeDef(td) if td.is_public => vec![td.name.clone()],
                TopLevel::Exception(ex) if ex.is_public => vec![ex.name.clone()],
                TopLevel::ExceptionGroup(eg) if eg.is_public => vec![eg.name.clone()],
                _ => vec![],
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

    fn handle_reimport(
        &mut self,
        import: &Import,
        import_span: &Span,
        cache: &ImportCache,
        caller_rename_map: &mut HashMap<String, String>,
    ) -> Result<(), HirBuildError> {
        if import.items.is_empty() {
            let alias = import
                .alias
                .clone()
                .unwrap_or_else(|| get_default_alias(&import.path));
            // cache.name_map values are canonical; map aliased access to canonical.
            for (original_name, canonical) in &cache.name_map {
                caller_rename_map.insert(format!("{}.{}", alias, original_name), canonical.clone());
            }
            return Ok(());
        }

        for item in &import.items {
            if !cache.public_names.contains(&item.name) {
                return Err(HirBuildError::ImportItemNotFound {
                    item: item.name.clone(),
                    path: import.path.clone(),
                    span: import_span.clone(),
                });
            }
            let visible = item.alias.as_ref().unwrap_or(&item.name);
            if let Some(canonical) = cache.name_map.get(&item.name) {
                caller_rename_map.insert(visible.clone(), canonical.clone());
            }
        }
        Ok(())
    }

    fn build_rename_map(
        &self,
        program: &Program,
        import: &Import,
        import_span: &Span,
    ) -> Result<HashMap<String, String>, HirBuildError> {
        let mut map = HashMap::new();
        let source = resolve_import_path(&import.path);
        let canonical = |name: &str| -> String { canonical_name(&source, name) };

        if import.items.is_empty() {
            for def in &program.definitions {
                match &def.node {
                    TopLevel::Let(gl) => {
                        map.insert(gl.name.clone(), canonical(&gl.name));
                    }
                    TopLevel::Port(port) => {
                        map.insert(port.name.clone(), canonical(&port.name));
                    }
                    _ => {}
                }
            }
            return Ok(map);
        }

        for item in &import.items {
            let found = program.definitions.iter().any(|def| match &def.node {
                TopLevel::Let(gl) if gl.is_public && gl.name == item.name => true,
                TopLevel::Port(port) if port.is_public && port.name == item.name => true,
                TopLevel::Enum(ed) if ed.is_public => {
                    ed.name == item.name || ed.variants.iter().any(|v| v.name == item.name)
                }
                TopLevel::TypeDef(td) if td.is_public && td.name == item.name => true,
                TopLevel::Exception(ex) if ex.is_public && ex.name == item.name => true,
                TopLevel::ExceptionGroup(eg) if eg.is_public && eg.name == item.name => true,
                _ => false,
            });
            if !found {
                return Err(HirBuildError::ImportItemNotFound {
                    item: item.name.clone(),
                    path: import.path.clone(),
                    span: import_span.clone(),
                });
            }
        }

        for def in &program.definitions {
            match &def.node {
                TopLevel::Let(gl) => {
                    map.insert(gl.name.clone(), canonical(&gl.name));
                }
                TopLevel::Port(port) => {
                    map.insert(port.name.clone(), canonical(&port.name));
                }
                _ => {}
            }
        }

        Ok(map)
    }

    fn rename(&self, name: &str, map: &HashMap<String, String>) -> String {
        if let Some(canonical) = map.get(name) {
            return canonical.clone();
        }
        // Longest-prefix match on '.': `X.Y` renames to `rename(X).Y` when X is
        // a rename-map key. Covers `Console.println`, `mod.Ctor`, etc.
        let mut best_idx: Option<usize> = None;
        for (i, _) in name.match_indices('.') {
            if map.contains_key(&name[..i]) {
                best_idx = Some(i);
            }
        }
        if let Some(i) = best_idx {
            let prefix = &name[..i];
            let suffix = &name[i + 1..];
            return format!("{}.{}", map[prefix], suffix);
        }
        name.to_string()
    }

    // ---- AST → MIR conversion (with port resolution & desugaring) ----

    fn convert_stmts(
        &mut self,
        stmts: &[Spanned<Stmt>],
        rename_map: &HashMap<String, String>,
        scope: &HandlerScope,
    ) -> Result<Vec<MirStmt>, HirBuildError> {
        let mut result = Vec::new();
        for (i, s) in stmts.iter().enumerate() {
            if let Stmt::LetPattern { pattern, value } = &s.node {
                // Desugar: remaining statements become the body of a single-case match
                let remaining = self.convert_stmts(&stmts[i + 1..], rename_map, scope)?;
                let case = MirMatchCase {
                    pattern: self.convert_pattern(&pattern.node),
                    body: remaining,
                };
                let match_expr = MirExpr::Match {
                    target: Box::new(self.convert_expr(value, rename_map, scope)?),
                    cases: vec![case],
                };
                result.push(MirStmt::Expr(match_expr));
                break;
            }
            result.extend(self.convert_stmt(&s.node, rename_map, scope)?);
        }
        Ok(result)
    }

    fn convert_stmt(
        &mut self,
        stmt: &Stmt,
        rename_map: &HashMap<String, String>,
        scope: &HandlerScope,
    ) -> Result<Vec<MirStmt>, HirBuildError> {
        match stmt {
            Stmt::Let {
                name,
                sigil,
                typ,
                value,
            } => {
                // Track variable types for closure capture resolution
                if let Some(t) = typ {
                    self.scope_var_types.insert(name.clone(), t.clone());
                }
                if matches!(sigil, Sigil::Lazy) {
                    // Desugar lazy binding: wrap value in a zero-arg closure (thunk)
                    let lifted_name = format!("__lambda_{}", self.lambda_counter);
                    self.lambda_counter += 1;

                    // Determine thunk return type from annotation
                    // Default to I64 (most common heap type) when no annotation;
                    // the actual type is resolved at LIR lowering via semantic_vars.
                    let ret_type = match typ {
                        Some(Type::Lazy(inner)) => *inner.clone(),
                        Some(t) => t.clone(),
                        None => Type::I64,
                    };

                    // Collect free variables from the value expression
                    let param_names: HashSet<String> = HashSet::new();
                    let mut known_names: HashSet<String> =
                        self.fn_ret_types.keys().cloned().collect();
                    for ext in &self.externals {
                        known_names.insert(ext.name.to_string());
                    }
                    for key in self.global_constants.keys() {
                        known_names.insert(key.clone());
                    }
                    // Also include rename-map keys: if a bare name renames to a
                    // known canonical, the bare name is NOT a free variable.
                    for (k, v) in rename_map {
                        if self.fn_ret_types.contains_key(v)
                            || self.global_constants.contains_key(v)
                        {
                            known_names.insert(k.clone());
                            known_names.insert(v.clone());
                        }
                    }
                    let thunk_body = vec![Spanned {
                        node: Stmt::Return(value.clone()),
                        span: value.span.clone(),
                    }];
                    let captures = collect_ast_free_vars(&thunk_body, &param_names, &known_names);

                    // Build capture params
                    let capture_params: Vec<MirParam> = captures
                        .iter()
                        .map(|cap_name| {
                            let cap_typ = self
                                .scope_var_types
                                .get(cap_name)
                                .cloned()
                                .unwrap_or(Type::I64);
                            MirParam {
                                name: Symbol::from(cap_name.as_str()),
                                label: Symbol::from(cap_name.as_str()),
                                typ: cap_typ,
                            }
                        })
                        .collect();

                    let arrow_type = Type::Arrow(
                        vec![],
                        Box::new(ret_type.clone()),
                        Box::new(Type::Row(vec![], None)),
                        Box::new(Type::Row(vec![], None)),
                    );
                    self.fn_ret_types
                        .insert(lifted_name.clone(), ret_type.clone());
                    self.fn_types.insert(lifted_name.clone(), arrow_type);

                    let capture_symbols: Vec<Symbol> =
                        captures.iter().map(|n| Symbol::from(n.as_str())).collect();

                    self.pending_functions.push(PendingFunction {
                        name: lifted_name.clone(),
                        params: capture_params,
                        ret_type,
                        body: thunk_body,
                        rename_map: rename_map.clone(),
                        source_file: self.current_source_file.clone(),
                        source_line: self.source_line_at(&value.span),
                        span: value.span.clone(),
                        preamble: vec![],
                        captures: capture_symbols.clone(),
                    });

                    // Always emit as Closure (even with no captures) because
                    // call_indirect always passes __env as first arg.
                    let mir_expr = MirExpr::Closure {
                        func: Symbol::from(lifted_name),
                        captures: capture_symbols,
                    };

                    return Ok(vec![MirStmt::Let {
                        name: Symbol::from(name.as_str()),
                        typ: typ.clone().unwrap_or(Type::Unit),
                        expr: mir_expr,
                    }]);
                }
                Ok(vec![MirStmt::Let {
                    name: Symbol::from(name.as_str()),
                    typ: typ.clone().unwrap_or(Type::Unit),
                    expr: self.convert_expr(value, rename_map, scope)?,
                }])
            }
            Stmt::Expr(expr) => {
                // Desugar for-loop at statement level
                if let Expr::For {
                    var,
                    start,
                    end_expr,
                    body,
                } = &expr.node
                {
                    let mir_start = self.convert_expr(start, rename_map, scope)?;
                    let mir_end = self.convert_expr(end_expr, rename_map, scope)?;
                    let mut mir_body = self.convert_stmts(body, rename_map, scope)?;
                    let var_sym = Symbol::from(var.as_str());
                    let end_sym = Symbol::from(format!("__for_end_{}", var));
                    mir_body.push(MirStmt::Assign {
                        target: MirExpr::Variable(var_sym),
                        value: MirExpr::BinaryOp(
                            Box::new(MirExpr::Variable(var_sym)),
                            BinaryOp::Add,
                            Box::new(MirExpr::Literal(Literal::Int(1))),
                        ),
                    });
                    return Ok(vec![
                        MirStmt::Let {
                            name: var_sym,
                            typ: Type::I64,
                            expr: mir_start,
                        },
                        MirStmt::Let {
                            name: end_sym,
                            typ: Type::I64,
                            expr: mir_end,
                        },
                        MirStmt::Expr(MirExpr::While {
                            cond: Box::new(MirExpr::BinaryOp(
                                Box::new(MirExpr::Variable(var_sym)),
                                BinaryOp::Lt,
                                Box::new(MirExpr::Variable(end_sym)),
                            )),
                            body: mir_body,
                        }),
                    ]);
                }
                Ok(vec![MirStmt::Expr(
                    self.convert_expr(expr, rename_map, scope)?,
                )])
            }
            Stmt::Return(expr) => Ok(vec![MirStmt::Return(
                self.convert_expr(expr, rename_map, scope)?,
            )]),
            Stmt::Assign { target, value } => Ok(vec![MirStmt::Assign {
                target: self.convert_expr(target, rename_map, scope)?,
                value: self.convert_expr(value, rename_map, scope)?,
            }]),
            Stmt::Try { body, catch_arms } => {
                let mir_body = self.convert_stmts(body, rename_map, scope)?;
                // Check if this is a legacy single-arm catch with variable pattern
                if catch_arms.len() == 1 {
                    if let Pattern::Variable(name, _) = &catch_arms[0].pattern.node {
                        return Ok(vec![MirStmt::Try {
                            body: mir_body,
                            catch_param: Symbol::from(name.as_str()),
                            catch_body: self.convert_stmts(
                                &catch_arms[0].body,
                                rename_map,
                                scope,
                            )?,
                        }]);
                    }
                }
                // Multi-arm or non-variable pattern: desugar to catch __exn -> match __exn do ... end
                // Exception group expansion: if a catch pattern names a group, expand to one arm per member
                let exn_sym = Symbol::from("__exn");
                let mut mir_cases: Vec<MirMatchCase> = Vec::new();
                for arm in catch_arms {
                    let mir_body = self.convert_stmts(&arm.body, rename_map, scope)?;
                    if let Pattern::Constructor(name, fields) = &arm.pattern.node {
                        if let Some(members) = self.exception_groups.get(name.occ()) {
                            // Group pattern: expand to one case per member
                            for member in members.clone() {
                                mir_cases.push(MirMatchCase {
                                    pattern: self.convert_pattern(&Pattern::Constructor(
                                        RdrName::Unqual(member),
                                        fields.clone(),
                                    )),
                                    body: mir_body.clone(),
                                });
                            }
                            continue;
                        }
                    }
                    mir_cases.push(MirMatchCase {
                        pattern: self.convert_pattern(&arm.pattern.node),
                        body: mir_body,
                    });
                }
                let match_expr = MirExpr::Match {
                    target: Box::new(MirExpr::Variable(exn_sym)),
                    cases: mir_cases,
                };
                Ok(vec![MirStmt::Try {
                    body: mir_body,
                    catch_param: exn_sym,
                    catch_body: vec![MirStmt::Expr(match_expr)],
                }])
            }
            Stmt::Inject { handlers, body } => {
                // Inject activates handlers: push into scope and inline body
                let mut new_scope = scope.clone();
                for handler_name in handlers {
                    let renamed = self.rename(handler_name, rename_map);
                    if let Some(port_name) = self.handler_port_names.get(&renamed) {
                        new_scope.active.insert(port_name.clone(), renamed);
                    }
                }
                // Inline: return body stmts directly (no Inject wrapper)
                self.convert_stmts(body, rename_map, &new_scope)
            }
            // LetPattern is handled in convert_stmts before calling convert_stmt
            Stmt::LetPattern { .. } => {
                unreachable!("LetPattern should be handled in convert_stmts")
            }
        }
    }

    fn convert_expr(
        &mut self,
        expr: &Spanned<Expr>,
        rename_map: &HashMap<String, String>,
        scope: &HandlerScope,
    ) -> Result<MirExpr, HirBuildError> {
        match &expr.node {
            Expr::Literal(lit) => Ok(MirExpr::Literal(lit.clone())),
            Expr::Variable(name, sigil) => {
                let name = name.as_dotted();
                let resolved = self.rename(&name, rename_map);
                if let Some(lit) = self.global_constants.get(&resolved) {
                    Ok(MirExpr::Literal(lit.clone()))
                } else if self.fn_ret_types.contains_key(&resolved) {
                    // Function name used as a value → emit FuncRef
                    Ok(MirExpr::FuncRef(Symbol::from(resolved)))
                } else {
                    let var = MirExpr::Variable(Symbol::from(resolved));
                    if matches!(sigil, Sigil::Lazy) {
                        // @x: force the lazy thunk (call the closure)
                        Ok(MirExpr::Force(Box::new(var)))
                    } else {
                        Ok(var)
                    }
                }
            }
            Expr::BinaryOp(lhs, op, rhs) => Ok(MirExpr::BinaryOp(
                Box::new(self.convert_expr(lhs, rename_map, scope)?),
                *op,
                Box::new(self.convert_expr(rhs, rename_map, scope)?),
            )),
            Expr::Borrow(name, _sigil) => {
                Ok(MirExpr::Borrow(Symbol::from(self.rename(name, rename_map))))
            }
            Expr::Call { func, args } => {
                let func = func.as_dotted();
                let renamed_func = self.rename(&func, rename_map);

                // Check if this is a port-qualified call (e.g., "Console.print")
                let port_match = self.port_methods.keys().find_map(|port_name| {
                    renamed_func
                        .strip_prefix(port_name.as_str())
                        .and_then(|s| s.strip_prefix('.'))
                        .map(|method_name| (port_name.clone(), method_name.to_string()))
                });
                if let Some((port_name, method_name)) = port_match {
                    return self.resolve_port_call(
                        &port_name,
                        &method_name,
                        args,
                        rename_map,
                        scope,
                        &expr.span,
                    );
                }

                let mir_args: Vec<(Symbol, MirExpr)> = args
                    .iter()
                    .map(|(label, e)| {
                        Ok((
                            Symbol::from(label.as_str()),
                            self.convert_expr(e, rename_map, scope)?,
                        ))
                    })
                    .collect::<Result<_, HirBuildError>>()?;

                let is_known_function = self.fn_ret_types.contains_key(&renamed_func)
                    || self
                        .externals
                        .iter()
                        .any(|e| e.name == renamed_func.as_str());

                if is_known_function {
                    let ret_type = self.lookup_ret_type(&renamed_func);
                    Ok(MirExpr::Call {
                        func: Symbol::from(renamed_func),
                        args: mir_args,
                        ret_type,
                    })
                } else {
                    // Dynamic call — callee is a variable holding a funcref
                    // callee_type is I64 here; actual Arrow type resolved in LIR lowering via semantic_vars
                    Ok(MirExpr::CallIndirect {
                        callee: Box::new(MirExpr::Variable(Symbol::from(renamed_func.as_str()))),
                        args: mir_args,
                        ret_type: Type::I64,
                        callee_type: Type::I64,
                    })
                }
            }
            Expr::Constructor(name, args) => {
                let ctor_name = name.occ();
                if args.is_empty() {
                    let resolved = self.rename(ctor_name, rename_map);
                    if let Some(lit) = self.global_constants.get(&resolved) {
                        return Ok(MirExpr::Literal(lit.clone()));
                    }
                }
                let mir_args: Vec<(Option<Symbol>, MirExpr)> = args
                    .iter()
                    .map(|(label, e)| {
                        Ok((
                            label.as_ref().map(|l| Symbol::from(l.as_str())),
                            self.convert_expr(e, rename_map, scope)?,
                        ))
                    })
                    .collect::<Result<_, HirBuildError>>()?;
                Ok(MirExpr::Constructor {
                    name: Symbol::from(ctor_name),
                    args: mir_args,
                })
            }
            Expr::Record(fields) => {
                let mir_fields: Vec<(Symbol, MirExpr)> = fields
                    .iter()
                    .map(|(n, e)| {
                        Ok((
                            Symbol::from(n.as_str()),
                            self.convert_expr(e, rename_map, scope)?,
                        ))
                    })
                    .collect::<Result<_, HirBuildError>>()?;
                Ok(MirExpr::Record(mir_fields))
            }
            Expr::Array(items) => {
                let mir_items: Vec<MirExpr> = items
                    .iter()
                    .map(|e| self.convert_expr(e, rename_map, scope))
                    .collect::<Result<_, _>>()?;
                Ok(MirExpr::Array(mir_items))
            }
            Expr::List(items) => {
                // Desugar [a, b, c] → Cons(a, Cons(b, Cons(c, Nil)))
                let mut acc = MirExpr::Constructor {
                    name: Symbol::from("Nil"),
                    args: vec![],
                };
                for item in items.iter().rev() {
                    acc = MirExpr::Constructor {
                        name: Symbol::from("Cons"),
                        args: vec![
                            (None, self.convert_expr(item, rename_map, scope)?),
                            (None, acc),
                        ],
                    };
                }
                Ok(acc)
            }
            Expr::Index(arr, idx) => Ok(MirExpr::Index(
                Box::new(self.convert_expr(arr, rename_map, scope)?),
                Box::new(self.convert_expr(idx, rename_map, scope)?),
            )),
            Expr::FieldAccess(e, field) => Ok(MirExpr::FieldAccess(
                Box::new(self.convert_expr(e, rename_map, scope)?),
                Symbol::from(field.as_str()),
            )),
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => Ok(MirExpr::If {
                cond: Box::new(self.convert_expr(cond, rename_map, scope)?),
                then_body: self.convert_stmts(then_branch, rename_map, scope)?,
                else_body: else_branch
                    .as_ref()
                    .map(|b| self.convert_stmts(b, rename_map, scope))
                    .transpose()?,
            }),
            Expr::Match { target, cases } => {
                let mir_cases: Vec<MirMatchCase> = cases
                    .iter()
                    .map(|c| {
                        Ok(MirMatchCase {
                            pattern: self.convert_pattern(&c.pattern.node),
                            body: self.convert_stmts(&c.body, rename_map, scope)?,
                        })
                    })
                    .collect::<Result<_, HirBuildError>>()?;
                Ok(MirExpr::Match {
                    target: Box::new(self.convert_expr(target, rename_map, scope)?),
                    cases: mir_cases,
                })
            }
            Expr::While { cond, body } => Ok(MirExpr::While {
                cond: Box::new(self.convert_expr(cond, rename_map, scope)?),
                body: self.convert_stmts(body, rename_map, scope)?,
            }),
            Expr::For {
                var,
                start: _,
                end_expr: _,
                body,
            } => {
                // For in expression position (unreachable in practice, desugared at stmt level)
                let var_sym = Symbol::from(var.as_str());
                let end_sym = Symbol::from(format!("__for_end_{}", var));
                Ok(MirExpr::While {
                    cond: Box::new(MirExpr::BinaryOp(
                        Box::new(MirExpr::Variable(var_sym)),
                        BinaryOp::Lt,
                        Box::new(MirExpr::Variable(end_sym)),
                    )),
                    body: {
                        let mut mir_body = self.convert_stmts(body, rename_map, scope)?;
                        mir_body.push(MirStmt::Assign {
                            target: MirExpr::Variable(var_sym),
                            value: MirExpr::BinaryOp(
                                Box::new(MirExpr::Variable(var_sym)),
                                BinaryOp::Add,
                                Box::new(MirExpr::Literal(Literal::Int(1))),
                            ),
                        });
                        mir_body
                    },
                })
            }
            Expr::Lambda {
                type_params: _,
                params,
                ret_type,
                requires,
                throws,
                body,
            } => {
                // Lambda-lift to a top-level function
                let lifted_name = format!("__lambda_{}", self.lambda_counter);
                self.lambda_counter += 1;

                // Collect free variables (captures) from the lambda body
                let param_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
                let mut known_names: HashSet<String> = self.fn_ret_types.keys().cloned().collect();
                for ext in &self.externals {
                    known_names.insert(ext.name.to_string());
                }
                for key in self.global_constants.keys() {
                    known_names.insert(key.clone());
                }
                // Also include rename-map keys: if a bare name renames to a
                // known canonical, the bare name is NOT a free variable.
                for (k, v) in rename_map {
                    if self.fn_ret_types.contains_key(v) || self.global_constants.contains_key(v) {
                        known_names.insert(k.clone());
                        known_names.insert(v.clone());
                    }
                }
                let captures = collect_ast_free_vars(body, &param_names, &known_names);

                // Build capture params (prepended before original params)
                let capture_params: Vec<MirParam> = captures
                    .iter()
                    .map(|name| {
                        let typ = self.scope_var_types.get(name).cloned().unwrap_or(Type::I64);
                        MirParam {
                            name: Symbol::from(name.as_str()),
                            label: Symbol::from(name.as_str()),
                            typ,
                        }
                    })
                    .collect();

                let mut all_params = capture_params;
                all_params.extend(params.iter().map(|p| MirParam {
                    name: Symbol::from(&p.name),
                    label: Symbol::from(&p.name),
                    typ: p.typ.clone(),
                }));

                let arrow_type = Type::Arrow(
                    params
                        .iter()
                        .map(|p| (p.name.clone(), p.typ.clone()))
                        .collect(),
                    Box::new(ret_type.clone()),
                    Box::new(requires.clone()),
                    Box::new(throws.clone()),
                );
                self.fn_ret_types
                    .insert(lifted_name.clone(), ret_type.clone());
                self.fn_types.insert(lifted_name.clone(), arrow_type);

                let capture_symbols: Vec<Symbol> =
                    captures.iter().map(|n| Symbol::from(n.as_str())).collect();

                self.pending_functions.push(PendingFunction {
                    name: lifted_name.clone(),
                    params: all_params,
                    ret_type: ret_type.clone(),
                    body: body.clone(),
                    rename_map: rename_map.clone(),
                    source_file: self.current_source_file.clone(),
                    source_line: self.source_line_at(&expr.span),
                    span: expr.span.clone(),
                    preamble: vec![],
                    captures: capture_symbols.clone(),
                });

                if captures.is_empty() {
                    Ok(MirExpr::FuncRef(Symbol::from(lifted_name)))
                } else {
                    Ok(MirExpr::Closure {
                        func: Symbol::from(lifted_name),
                        captures: capture_symbols,
                    })
                }
            }
            Expr::Raise(e) => Ok(MirExpr::Raise(Box::new(
                self.convert_expr(e, rename_map, scope)?,
            ))),
            Expr::Force(e) => Ok(MirExpr::Force(Box::new(
                self.convert_expr(e, rename_map, scope)?,
            ))),
            Expr::External(sym, _tparams, _typ) => {
                Ok(MirExpr::Variable(Symbol::from(sym.as_str())))
            }
            Expr::Handler { .. } => {
                // Handler expressions collected during process_program
                Ok(MirExpr::Literal(Literal::Unit))
            }
        }
    }

    /// Resolve a port method call to a direct call to the handler function.
    fn resolve_port_call(
        &mut self,
        port_name: &str,
        method_name: &str,
        args: &[(String, Spanned<Expr>)],
        rename_map: &HashMap<String, String>,
        scope: &HandlerScope,
        span: &Span,
    ) -> Result<MirExpr, HirBuildError> {
        let handler_name =
            scope
                .active
                .get(port_name)
                .ok_or_else(|| HirBuildError::UnresolvedPort {
                    port: port_name.to_string(),
                    method: method_name.to_string(),
                    span: span.clone(),
                })?;

        let func_name = handler_func_name(handler_name, method_name);
        let ret_type = self.lookup_ret_type(&func_name);

        let mir_args: Vec<(Symbol, MirExpr)> = args
            .iter()
            .map(|(label, e)| {
                Ok((
                    Symbol::from(label.as_str()),
                    self.convert_expr(e, rename_map, scope)?,
                ))
            })
            .collect::<Result<_, HirBuildError>>()?;

        Ok(MirExpr::Call {
            func: Symbol::from(func_name),
            args: mir_args,
            ret_type,
        })
    }

    fn convert_pattern(&self, pattern: &Pattern) -> MirPattern {
        match pattern {
            Pattern::Literal(lit) => MirPattern::Literal(lit.clone()),
            Pattern::Variable(name, sigil) => {
                MirPattern::Variable(Symbol::from(name.as_str()), sigil.clone())
            }
            Pattern::Constructor(name, fields) => MirPattern::Constructor {
                name: Symbol::from(name.occ()),
                fields: fields
                    .iter()
                    .map(|(label, p)| {
                        (
                            label.as_ref().map(|l| Symbol::from(l.as_str())),
                            self.convert_pattern(&p.node),
                        )
                    })
                    .collect(),
            },
            Pattern::Record(fields, open) => MirPattern::Record(
                fields
                    .iter()
                    .map(|(n, p)| (Symbol::from(n.as_str()), self.convert_pattern(&p.node)))
                    .collect(),
                *open,
            ),
            Pattern::Wildcard => MirPattern::Wildcard,
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

/// Collect function names called in MIR statements (for reachability)
fn collect_calls_in_mir_stmts(stmts: &[MirStmt], out: &mut Vec<Symbol>) {
    for stmt in stmts {
        match stmt {
            MirStmt::Let { expr, .. } => collect_calls_in_mir_expr(expr, out),
            MirStmt::Expr(expr) => collect_calls_in_mir_expr(expr, out),
            MirStmt::Return(expr) => collect_calls_in_mir_expr(expr, out),
            MirStmt::Assign { target, value } => {
                collect_calls_in_mir_expr(target, out);
                collect_calls_in_mir_expr(value, out);
            }
            MirStmt::Try {
                body, catch_body, ..
            } => {
                collect_calls_in_mir_stmts(body, out);
                collect_calls_in_mir_stmts(catch_body, out);
            }
        }
    }
}

fn collect_calls_in_mir_expr(expr: &MirExpr, out: &mut Vec<Symbol>) {
    match expr {
        MirExpr::Call { func, args, .. } => {
            out.push(*func);
            for (_, arg) in args {
                collect_calls_in_mir_expr(arg, out);
            }
        }
        MirExpr::BinaryOp(lhs, _, rhs) => {
            collect_calls_in_mir_expr(lhs, out);
            collect_calls_in_mir_expr(rhs, out);
        }
        MirExpr::Constructor { args, .. } => {
            for (_, arg) in args {
                collect_calls_in_mir_expr(arg, out);
            }
        }
        MirExpr::Record(fields) => {
            for (_, expr) in fields {
                collect_calls_in_mir_expr(expr, out);
            }
        }
        MirExpr::Array(items) => {
            for item in items {
                collect_calls_in_mir_expr(item, out);
            }
        }
        MirExpr::Index(arr, idx) => {
            collect_calls_in_mir_expr(arr, out);
            collect_calls_in_mir_expr(idx, out);
        }
        MirExpr::FieldAccess(expr, _) => collect_calls_in_mir_expr(expr, out),
        MirExpr::If {
            cond,
            then_body,
            else_body,
        } => {
            collect_calls_in_mir_expr(cond, out);
            collect_calls_in_mir_stmts(then_body, out);
            if let Some(else_stmts) = else_body {
                collect_calls_in_mir_stmts(else_stmts, out);
            }
        }
        MirExpr::Match { target, cases } => {
            collect_calls_in_mir_expr(target, out);
            for case in cases {
                collect_calls_in_mir_stmts(&case.body, out);
            }
        }
        MirExpr::While { cond, body } => {
            collect_calls_in_mir_expr(cond, out);
            collect_calls_in_mir_stmts(body, out);
        }
        MirExpr::Raise(expr) | MirExpr::Force(expr) => collect_calls_in_mir_expr(expr, out),
        MirExpr::FuncRef(name) => out.push(*name),
        MirExpr::Closure { func, .. } => out.push(*func),
        MirExpr::CallIndirect { callee, args, .. } => {
            collect_calls_in_mir_expr(callee, out);
            for (_, arg) in args {
                collect_calls_in_mir_expr(arg, out);
            }
        }
        MirExpr::Borrow(_) | MirExpr::Literal(_) | MirExpr::Variable(_) => {}
    }
}

/// Collect free variables in an AST statement block.
/// Returns variable names that are referenced but not locally defined.
fn collect_ast_free_vars(
    body: &[Spanned<Stmt>],
    params: &HashSet<String>,
    known_names: &HashSet<String>,
) -> Vec<String> {
    let mut defined = params.clone();
    let mut referenced = Vec::new();
    let mut seen = HashSet::new();
    for stmt in body {
        collect_ast_stmt_refs(
            &stmt.node,
            &mut defined,
            &mut referenced,
            &mut seen,
            known_names,
        );
    }
    referenced
}

fn collect_ast_stmt_refs(
    stmt: &Stmt,
    defined: &mut HashSet<String>,
    referenced: &mut Vec<String>,
    seen: &mut HashSet<String>,
    known: &HashSet<String>,
) {
    match stmt {
        Stmt::Let { name, value, .. } => {
            collect_ast_expr_refs(&value.node, defined, referenced, seen, known);
            defined.insert(name.clone());
        }
        Stmt::Expr(expr) | Stmt::Return(expr) => {
            collect_ast_expr_refs(&expr.node, defined, referenced, seen, known);
        }
        Stmt::Assign { target, value } => {
            collect_ast_expr_refs(&target.node, defined, referenced, seen, known);
            collect_ast_expr_refs(&value.node, defined, referenced, seen, known);
        }
        Stmt::Try { body, catch_arms } => {
            for s in body {
                collect_ast_stmt_refs(&s.node, defined, referenced, seen, known);
            }
            for arm in catch_arms {
                if let Pattern::Variable(name, _) = &arm.pattern.node {
                    defined.insert(name.clone());
                }
                for s in &arm.body {
                    collect_ast_stmt_refs(&s.node, defined, referenced, seen, known);
                }
            }
        }
        Stmt::Inject { body, .. } => {
            for s in body {
                collect_ast_stmt_refs(&s.node, defined, referenced, seen, known);
            }
        }
        Stmt::LetPattern { pattern, value } => {
            collect_ast_expr_refs(&value.node, defined, referenced, seen, known);
            collect_ast_pattern_defs(&pattern.node, defined);
        }
    }
}

fn collect_ast_expr_refs(
    expr: &Expr,
    defined: &HashSet<String>,
    referenced: &mut Vec<String>,
    seen: &mut HashSet<String>,
    known: &HashSet<String>,
) {
    match expr {
        Expr::Variable(name, _) => {
            let name = name.as_dotted();
            if !defined.contains(&name) && !known.contains(&name) && seen.insert(name.clone()) {
                referenced.push(name);
            }
        }
        Expr::Borrow(name, _) => {
            if !defined.contains(name) && !known.contains(name) && seen.insert(name.clone()) {
                referenced.push(name.clone());
            }
        }
        Expr::BinaryOp(lhs, _, rhs) => {
            collect_ast_expr_refs(&lhs.node, defined, referenced, seen, known);
            collect_ast_expr_refs(&rhs.node, defined, referenced, seen, known);
        }
        Expr::Call { func, args } => {
            let func = func.as_dotted();
            if !defined.contains(&func) && !known.contains(&func) && seen.insert(func.clone()) {
                referenced.push(func);
            }
            for (_, arg) in args {
                collect_ast_expr_refs(&arg.node, defined, referenced, seen, known);
            }
        }
        Expr::Constructor(_, args) => {
            for (_, arg) in args {
                collect_ast_expr_refs(&arg.node, defined, referenced, seen, known);
            }
        }
        Expr::Record(fields) => {
            for (_, e) in fields {
                collect_ast_expr_refs(&e.node, defined, referenced, seen, known);
            }
        }
        Expr::Array(items) | Expr::List(items) => {
            for item in items {
                collect_ast_expr_refs(&item.node, defined, referenced, seen, known);
            }
        }
        Expr::Index(arr, idx) => {
            collect_ast_expr_refs(&arr.node, defined, referenced, seen, known);
            collect_ast_expr_refs(&idx.node, defined, referenced, seen, known);
        }
        Expr::FieldAccess(expr, _) => {
            collect_ast_expr_refs(&expr.node, defined, referenced, seen, known);
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_ast_expr_refs(&cond.node, defined, referenced, seen, known);
            for s in then_branch {
                collect_ast_stmt_refs(&s.node, &mut defined.clone(), referenced, seen, known);
            }
            if let Some(else_stmts) = else_branch {
                for s in else_stmts {
                    collect_ast_stmt_refs(&s.node, &mut defined.clone(), referenced, seen, known);
                }
            }
        }
        Expr::Match { target, cases } => {
            collect_ast_expr_refs(&target.node, defined, referenced, seen, known);
            for case in cases {
                let mut case_defined = defined.clone();
                collect_ast_pattern_defs(&case.pattern.node, &mut case_defined);
                for s in &case.body {
                    collect_ast_stmt_refs(&s.node, &mut case_defined, referenced, seen, known);
                }
            }
        }
        Expr::While { cond, body } => {
            collect_ast_expr_refs(&cond.node, defined, referenced, seen, known);
            for s in body {
                collect_ast_stmt_refs(&s.node, &mut defined.clone(), referenced, seen, known);
            }
        }
        Expr::For {
            var,
            start,
            end_expr,
            body,
        } => {
            collect_ast_expr_refs(&start.node, defined, referenced, seen, known);
            collect_ast_expr_refs(&end_expr.node, defined, referenced, seen, known);
            let mut inner_defined = defined.clone();
            inner_defined.insert(var.clone());
            for s in body {
                collect_ast_stmt_refs(&s.node, &mut inner_defined, referenced, seen, known);
            }
        }
        Expr::Lambda { params, body, .. } => {
            // Lambda introduces a new scope — params are local, don't leak
            let mut inner_defined = defined.clone();
            for p in params {
                inner_defined.insert(p.name.clone());
            }
            for s in body {
                collect_ast_stmt_refs(&s.node, &mut inner_defined, referenced, seen, known);
            }
        }
        Expr::Raise(e) | Expr::Force(e) => {
            collect_ast_expr_refs(&e.node, defined, referenced, seen, known);
        }
        Expr::Literal(_) | Expr::External(_, _, _) | Expr::Handler { .. } => {}
    }
}

fn collect_ast_pattern_defs(pattern: &Pattern, defined: &mut HashSet<String>) {
    match pattern {
        Pattern::Variable(name, _) => {
            defined.insert(name.clone());
        }
        Pattern::Constructor(_, fields) => {
            for (_, pat) in fields {
                collect_ast_pattern_defs(&pat.node, defined);
            }
        }
        Pattern::Record(fields, _) => {
            for (_, pat) in fields {
                collect_ast_pattern_defs(&pat.node, defined);
            }
        }
        Pattern::Wildcard | Pattern::Literal(_) => {}
    }
}

/// Map a resolved import path to a WIT module name if it refers to stdlib.wasm.
///
/// If the source file is a known stdlib module, uses its specific WIT interface.
/// Otherwise falls back to `nexus:stdlib/bundle` — a catch-all for non-stdlib
/// files (e.g. nxc modules) that import from stdlib.wasm directly.
/// Non-stdlib imports (e.g. `nexus:runtime/backtrace`) pass through unchanged.
fn resolve_stdlib_wit_module(resolved: &str, wit_interface: Option<&str>) -> String {
    if resolved.ends_with("stdlib.wasm") {
        match wit_interface {
            Some(iface) => iface.to_string(),
            None => resolved.to_string(),
        }
    } else if resolved.ends_with(".nx") {
        // Import path is a .nx file (e.g. "stdlib/hashmap.nx") — use its stem for WIT mapping.
        let path = std::path::Path::new(resolved);
        stdlib_nx_to_wit_interface(path).unwrap_or_else(|| resolved.to_string())
    } else {
        resolved.to_string()
    }
}

/// Map a stdlib `.nx` source file path to its WIT interface name.
///
/// Externals from these files get `wasm_module` set to the WIT name
/// instead of the raw `nxlib/stdlib/stdlib.wasm` file path. This keeps
/// the surface language unchanged while the IR carries component-model
/// metadata matching the WIT definitions in `wit/nexus-stdlib/`.
/// Convert an `__nx_*` FFI function name to its WIT-canonical kebab-case form.
///
/// `__nx_abs_i64`   → `abs-i64`
/// `__nx_http_get`  → `http-get`
/// `__nx_string_from_char_code` → `string-from-char-code`
///
/// Non-`__nx_` names (e.g. `allocate`) are returned as-is.
fn wit_canonical_name(ffi_name: &str) -> String {
    if let Some(stripped) = ffi_name.strip_prefix("__nx_") {
        stripped.replace('_', "-")
    } else {
        ffi_name.to_string()
    }
}

fn stdlib_nx_to_wit_interface(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let wit = match stem {
        "math" => "nexus:stdlib/math",
        "string" => "nexus:stdlib/string-ops",
        "stdio" => "nexus:stdlib/stdio",
        "fs" => "nexus:stdlib/filesystem",
        "net" => "nexus:stdlib/network",
        "proc" => "nexus:stdlib/process",
        "env" => "nexus:stdlib/environment",
        "clock" => "nexus:stdlib/clock",
        "random" => "nexus:stdlib/random",
        "char" => "nexus:stdlib/string-ops",
        "bytebuffer" => "nexus:stdlib/bytebuffer",
        "hashmap" | "stringmap" | "set" | "array" => "nexus:stdlib/collections",
        "core" => "nexus:stdlib/core",
        _ => return None,
    };
    Some(wit.to_string())
}
