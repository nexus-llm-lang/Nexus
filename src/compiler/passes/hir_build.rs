//! AST → MIR (module flatten, name resolve, handler collect, port resolve)
//!
//! Single-pass transformation from parsed AST to MIR:
//! - Module resolution (imports, file loading, cycle detection)
//! - Name resolution (qualify identifiers, resolve constructors)
//! - Handler collection (synthesize handler functions)
//! - Port call resolution (static dispatch to handler functions)
//! - For-loop desugaring (→ while loop)
//! - Inject inlining (scope-based handler activation)

use crate::intern::Symbol;
use crate::ir::mir::*;
use crate::lang::ast::*;
use crate::lang::parser;
use crate::lang::stdlib::{load_stdlib_nx_programs, resolve_import_path};
use std::collections::{HashMap, HashSet};

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
        self.collect_declarations(program, &HashMap::new())?;

        // Pass 2: Convert all pending function bodies with full handler scope
        let scope = self.build_global_scope();
        let pending = std::mem::take(&mut self.pending_functions);
        for pf in pending {
            let mut body = pf.preamble;
            body.extend(self.convert_stmts(&pf.body, &pf.rename_map, &scope)?);
            self.functions.push(MirFunction {
                name: Symbol::from(&pf.name),
                params: pf.params,
                ret_type: pf.ret_type,
                body,
                span: pf.span,
                source_file: pf.source_file,
                source_line: pf.source_line,
            });
        }

        // Collect reachable functions from main
        let reachable = self.collect_reachable();

        // Filter to only reachable functions and externals
        let functions: Vec<MirFunction> = self
            .functions
            .iter()
            .filter(|f| reachable.contains(&f.name))
            .cloned()
            .collect();
        let externals: Vec<MirExternal> = self
            .externals
            .iter()
            .filter(|e| reachable.contains(&e.name))
            .cloned()
            .collect();

        Ok(MirProgram {
            functions,
            externals,
            enum_defs: self.enum_defs.clone(),
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
                                        self.externals.push(MirExternal {
                                            name: Symbol::from(&gl.name),
                                            wasm_module: Symbol::from(wasm_mod.as_str()),
                                            wasm_name: Symbol::from(wasm_name.as_str()),
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
                            requires: _,
                            throws: _,
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
                                    items: vec!["argv".to_string()],
                                    is_external: false,
                                };
                                self.process_import(&proc_import, &def.span)?;

                                let arg_name = params[0].name.clone();
                                let preamble = vec![MirStmt::Let {
                                    name: Symbol::from(&arg_name),
                                    typ: Type::List(Box::new(Type::String)),
                                    expr: MirExpr::Call {
                                        func: Symbol::from("argv"),
                                        args: vec![],
                                        ret_type: Type::List(Box::new(Type::String)),
                                    },
                                }];
                                self.fn_ret_types
                                    .insert(name.clone(), ret_type.clone());
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
                                });
                            } else {
                                self.fn_ret_types
                                    .insert(name.clone(), ret_type.clone());
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
                                });
                            }
                        }
                        Expr::External(wasm_name, _type_params, typ) => {
                            if let Type::Arrow(params, ret, _requires, throws) = typ {
                                if let Some(ref wasm_mod) = current_wasm_module {
                                    let name_sym = Symbol::from(&name);
                                    self.externals.retain(|e| e.name != name_sym);
                                    self.externals.push(MirExternal {
                                        name: name_sym,
                                        wasm_module: Symbol::from(wasm_mod.as_str()),
                                        wasm_name: Symbol::from(wasm_name.as_str()),
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
                                let synth_name =
                                    format!("__handler_{}_{}", name, hf.name);
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
                                });
                            }
                            self.handler_port_names
                                .insert(name.clone(), port_name.clone());
                            self.handler_bindings.insert(
                                name.clone(),
                                HandlerBinding { port_name },
                            );
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

    fn process_import(&mut self, import: &Import, import_span: &Span) -> Result<(), HirBuildError> {
        let resolved_path = resolve_import_path(&import.path);
        if self.import_stack.iter().any(|p| p == &resolved_path) {
            return Err(HirBuildError::CyclicImport {
                path: resolved_path,
                span: import_span.clone(),
            });
        }

        // Diamond import dedup
        if let Some(cache) = self.imported_modules.get(&resolved_path).cloned() {
            return self.handle_reimport(import, import_span, &cache);
        }

        self.import_stack.push(resolved_path.clone());
        let src =
            std::fs::read_to_string(&resolved_path).map_err(|e| HirBuildError::ImportReadError {
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

        // Save and swap source context for the imported module
        let saved_source_file = self.current_source_file.take();
        let saved_source_text = self.current_source_text.take();
        self.current_source_file = Some(resolved_path.clone());
        self.current_source_text = Some(src.clone());

        let rename_map = self.build_rename_map(&imported_program, import, import_span)?;
        let result = self.collect_declarations(&imported_program, &rename_map);

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
    ) -> Result<(), HirBuildError> {
        if import.items.is_empty() {
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
                    self.create_function_alias(item, registered_name);
                    self.create_external_alias(item, registered_name);
                }
            }
        }
        Ok(())
    }

    fn create_function_alias(&mut self, alias_name: &str, registered_name: &str) {
        // Check already-converted functions
        if let Some(func) = self
            .functions
            .iter()
            .find(|f| f.name == registered_name)
            .cloned()
        {
            self.functions.retain(|f| f.name != alias_name);
            let mut aliased = func;
            aliased.name = Symbol::from(alias_name);
            self.functions.push(aliased);
            return;
        }
        // Check pending functions (two-pass: bodies not yet converted)
        if let Some(pf) = self
            .pending_functions
            .iter()
            .find(|f| f.name == registered_name)
            .cloned()
        {
            self.pending_functions.retain(|f| f.name != alias_name);
            let mut aliased = pf;
            aliased.name = alias_name.to_string();
            if let Some(ty) = self.fn_ret_types.get(registered_name).cloned() {
                self.fn_ret_types.insert(alias_name.to_string(), ty);
            }
            self.pending_functions.push(aliased);
        }
    }

    fn create_external_alias(&mut self, alias_name: &str, registered_name: &str) {
        if let Some(ext) = self
            .externals
            .iter()
            .find(|e| e.name == registered_name)
            .cloned()
        {
            self.externals.retain(|e| e.name != alias_name);
            let mut aliased = ext;
            aliased.name = Symbol::from(alias_name);
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

        let selected: HashSet<String> = import.items.iter().cloned().collect();
        for item in &import.items {
            let found = program.definitions.iter().any(|def| match &def.node {
                TopLevel::Let(gl) if gl.is_public && gl.name == *item => true,
                TopLevel::Port(port) if port.is_public && port.name == *item => true,
                TopLevel::Enum(ed) if ed.is_public => {
                    ed.name == *item || ed.variants.iter().any(|v| v.name == *item)
                }
                TopLevel::TypeDef(td) if td.is_public && td.name == *item => true,
                TopLevel::Exception(ex) if ex.is_public && ex.name == *item => true,
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
                let path_hash =
                    crate::compiler::type_tag::fnv1a_hash(&[import.path.as_bytes()]) % 10000;
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

    // ---- AST → MIR conversion (with port resolution & desugaring) ----

    fn convert_stmts(
        &self,
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
        &self,
        stmt: &Stmt,
        rename_map: &HashMap<String, String>,
        scope: &HandlerScope,
    ) -> Result<Vec<MirStmt>, HirBuildError> {
        match stmt {
            Stmt::Let {
                name, typ, value, ..
            } => Ok(vec![MirStmt::Let {
                name: Symbol::from(name.as_str()),
                typ: typ.clone().unwrap_or(Type::Unit),
                expr: self.convert_expr(value, rename_map, scope)?,
            }]),
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
            Stmt::Conc(tasks) => {
                let mir_tasks: Vec<MirFunction> = tasks
                    .iter()
                    .map(|t| {
                        let body = self.convert_stmts(&t.body, rename_map, scope)?;
                        Ok(MirFunction {
                            name: Symbol::from(t.name.as_str()),
                            params: vec![],
                            ret_type: Type::Unit,
                            body,
                            span: 0..0,
                            source_file: None,
                            source_line: None,
                        })
                    })
                    .collect::<Result<_, HirBuildError>>()?;
                Ok(vec![MirStmt::Conc(mir_tasks)])
            }
            Stmt::Try {
                body,
                catch_param,
                catch_body,
            } => Ok(vec![MirStmt::Try {
                body: self.convert_stmts(body, rename_map, scope)?,
                catch_param: Symbol::from(catch_param.as_str()),
                catch_body: self.convert_stmts(catch_body, rename_map, scope)?,
            }]),
            Stmt::Inject { handlers, body } => {
                // Inject activates handlers: push into scope and inline body
                let mut new_scope = scope.clone();
                for handler_name in handlers {
                    let renamed = self.rename(handler_name, rename_map);
                    if let Some(port_name) = self.handler_port_names.get(&renamed) {
                        new_scope
                            .active
                            .insert(port_name.clone(), renamed);
                    }
                }
                // Inline: return body stmts directly (no Inject wrapper)
                self.convert_stmts(body, rename_map, &new_scope)
            }
            // LetPattern is handled in convert_stmts before calling convert_stmt
            Stmt::LetPattern { .. } => unreachable!("LetPattern should be handled in convert_stmts"),
        }
    }

    fn convert_expr(
        &self,
        expr: &Spanned<Expr>,
        rename_map: &HashMap<String, String>,
        scope: &HandlerScope,
    ) -> Result<MirExpr, HirBuildError> {
        match &expr.node {
            Expr::Literal(lit) => Ok(MirExpr::Literal(lit.clone())),
            Expr::Variable(name, _sigil) => {
                let resolved = self.rename(name, rename_map);
                if let Some(lit) = self.global_constants.get(&resolved) {
                    Ok(MirExpr::Literal(lit.clone()))
                } else {
                    Ok(MirExpr::Variable(Symbol::from(resolved)))
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
                let renamed_func = self.rename(func, rename_map);

                // Check if this is a port-qualified call (e.g., "Console.print")
                for (port_name, _) in &self.port_methods {
                    if let Some(method_name) = renamed_func
                        .strip_prefix(port_name.as_str())
                        .and_then(|s| s.strip_prefix('.'))
                    {
                        return self.resolve_port_call(
                            port_name, method_name, args, rename_map, scope, &expr.span,
                        );
                    }
                }

                let mir_args: Vec<(Symbol, MirExpr)> = args
                    .iter()
                    .map(|(label, e)| {
                        Ok((Symbol::from(label.as_str()), self.convert_expr(e, rename_map, scope)?))
                    })
                    .collect::<Result<_, HirBuildError>>()?;

                let ret_type = self.lookup_ret_type(&renamed_func);

                Ok(MirExpr::Call {
                    func: Symbol::from(renamed_func),
                    args: mir_args,
                    ret_type,
                })
            }
            Expr::Constructor(name, args) => {
                if args.is_empty() {
                    let resolved = self.rename(name, rename_map);
                    if let Some(lit) = self.global_constants.get(&resolved) {
                        return Ok(MirExpr::Literal(lit.clone()));
                    }
                }
                let mir_args: Vec<(Option<Symbol>, MirExpr)> = args
                    .iter()
                    .map(|(label, e)| {
                        Ok((label.as_ref().map(|l| Symbol::from(l.as_str())), self.convert_expr(e, rename_map, scope)?))
                    })
                    .collect::<Result<_, HirBuildError>>()?;
                Ok(MirExpr::Constructor {
                    name: Symbol::from(name.as_str()),
                    args: mir_args,
                })
            }
            Expr::Record(fields) => {
                let mir_fields: Vec<(Symbol, MirExpr)> = fields
                    .iter()
                    .map(|(n, e)| Ok((Symbol::from(n.as_str()), self.convert_expr(e, rename_map, scope)?)))
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
                        args: vec![(None, self.convert_expr(item, rename_map, scope)?), (None, acc)],
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
            Expr::Lambda { .. } => {
                // Lambda in expression position — emit unit placeholder
                Ok(MirExpr::Literal(Literal::Unit))
            }
            Expr::Raise(e) => Ok(MirExpr::Raise(Box::new(
                self.convert_expr(e, rename_map, scope)?,
            ))),
            Expr::External(sym, _tparams, _typ) => Ok(MirExpr::Variable(Symbol::from(sym.as_str()))),
            Expr::Handler { .. } => {
                // Handler expressions collected during process_program
                Ok(MirExpr::Literal(Literal::Unit))
            }
        }
    }

    /// Resolve a port method call to a direct call to the handler function.
    fn resolve_port_call(
        &self,
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

        let func_name = format!("__handler_{}_{}", handler_name, method_name);
        let ret_type = self.lookup_ret_type(&func_name);

        let mir_args: Vec<(Symbol, MirExpr)> = args
            .iter()
            .map(|(label, e)| Ok((Symbol::from(label.as_str()), self.convert_expr(e, rename_map, scope)?)))
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
            Pattern::Variable(name, sigil) => MirPattern::Variable(Symbol::from(name.as_str()), sigil.clone()),
            Pattern::Constructor(name, fields) => MirPattern::Constructor {
                name: Symbol::from(name.as_str()),
                fields: fields
                    .iter()
                    .map(|(label, p)| (label.as_ref().map(|l| Symbol::from(l.as_str())), self.convert_pattern(&p.node)))
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
            MirStmt::Conc(tasks) => {
                for task in tasks {
                    collect_calls_in_mir_stmts(&task.body, out);
                }
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
        MirExpr::Raise(expr) => collect_calls_in_mir_expr(expr, out),
        MirExpr::Borrow(_) | MirExpr::Literal(_) | MirExpr::Variable(_) => {}
    }
}
