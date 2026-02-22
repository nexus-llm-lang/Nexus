use chumsky::Parser;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;

use crate::lang::ast::{
    EnumDef, ExceptionDef, Expr, Function, Import, Literal, MatchCase, Pattern, Program, Sigil,
    Span, Spanned, Stmt, TopLevel, Type,
};
use crate::lang::parser;
use crate::lang::stdlib::load_stdlib_nx_programs;

use super::anf::{AnfAtom, AnfExpr, AnfExternal, AnfFunction, AnfParam, AnfProgram, AnfStmt};

#[derive(Debug, Clone, PartialEq)]
pub struct LowerError {
    pub message: String,
    pub span: Option<Span>,
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.span {
            Some(span) => write!(f, "{} at {:?}", self.message, span),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for LowerError {}

#[derive(Debug, Clone)]
struct Signature {
    params: Vec<(String, Type)>,
    ret: Type,
    effects: Type,
    is_generic: bool,
}

#[derive(Debug, Clone)]
pub struct ExternalBinding {
    pub name: String,
    pub wasm_name: String,
    pub typ: Type,
    pub wasm_module: Option<String>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct CollectedDefinitions {
    pub definitions: Vec<Spanned<TopLevel>>,
    pub external_bindings: HashMap<String, ExternalBinding>,
}

/// Lowers a typed Nexus program into typed ANF for wasm code generation.
pub fn lower_to_typed_anf(program: &Program) -> Result<AnfProgram, LowerError> {
    let mut functions: HashMap<String, Function> = HashMap::new();
    let mut signatures: HashMap<String, Signature> = HashMap::new();
    let mut enums: HashMap<String, EnumDef> = HashMap::new();
    let mut exceptions: HashMap<String, ExceptionDef> = HashMap::new();
    let collected = collect_all_definitions(program)?;
    let all_definitions = &collected.definitions;

    for def in all_definitions {
        match &def.node {
            TopLevel::Let(gl) => match &gl.value.node {
                Expr::Lambda {
                    type_params,
                    params,
                    ret_type,
                    effects,
                    body,
                } => {
                    signatures.insert(
                        gl.name.clone(),
                        Signature {
                            params: params
                                .iter()
                                .map(|p| (p.name.clone(), p.typ.clone()))
                                .collect(),
                            ret: ret_type.clone(),
                            effects: effects.clone(),
                            is_generic: !type_params.is_empty(),
                        },
                    );
                    functions.insert(
                        gl.name.clone(),
                        Function {
                            name: gl.name.clone(),
                            is_public: gl.is_public,
                            type_params: type_params.clone(),
                            params: params.clone(),
                            ret_type: ret_type.clone(),
                            effects: effects.clone(),
                            body: body.clone(),
                        },
                    );
                }
                Expr::External(_, typ) => {
                    if let Type::Arrow(params, ret, effects) = typ {
                        signatures.insert(
                            gl.name.clone(),
                            Signature {
                                params: params.clone(),
                                ret: *ret.clone(),
                                effects: *effects.clone(),
                                is_generic: false,
                            },
                        );
                    }
                }
                _ => {}
            },
            TopLevel::Enum(ed) => {
                enums.insert(ed.name.clone(), ed.clone());
            }
            TopLevel::Exception(ex) => {
                exceptions.insert(ex.name.clone(), ex.clone());
            }
            _ => {}
        }
    }

    let reachable = collect_reachable_functions(&functions, &collected.external_bindings);
    if !reachable.contains("main") {
        return Err(LowerError {
            message: "main function not found".to_string(),
            span: None,
        });
    }

    let mut lowered = Vec::new();
    for def in all_definitions {
        if let TopLevel::Let(gl) = &def.node {
            if !reachable.contains(&gl.name) {
                continue;
            }
            if let Expr::Lambda { type_params, .. } = &gl.value.node {
                if !type_params.is_empty() {
                    return Err(LowerError {
                        message: format!(
                            "reachable generic function '{}' is not supported in wasm ANF lowering yet",
                            gl.name
                        ),
                        span: Some(def.span.clone()),
                    });
                }
                let mut ctx = LowerCtx::new(&signatures, &enums, &exceptions);
                lowered.push(ctx.lower_function(functions.get(&gl.name).unwrap())?);
            }
        }
    }

    let mut reachable_externals = HashSet::new();
    for name in &reachable {
        if collected.external_bindings.contains_key(name) {
            reachable_externals.insert(name.clone());
        }
    }

    let mut external_names: Vec<String> = reachable_externals.into_iter().collect();
    external_names.sort();
    let mut lowered_externals = Vec::new();
    for name in external_names {
        let binding = collected
            .external_bindings
            .get(&name)
            .ok_or_else(|| LowerError {
                message: format!("external binding '{}' not found", name),
                span: None,
            })?;
        let wasm_module = binding.wasm_module.clone().ok_or_else(|| LowerError {
            message: format!(
                "external function '{}' requires a preceding 'import external ...' in the same module",
                name
            ),
            span: Some(binding.span.clone()),
        })?;
        if !std::path::Path::new(&wasm_module).exists() {
            return Err(LowerError {
                message: format!(
                    "external module '{}' not found (build it first, e.g. src/lib/core/build.sh)",
                    wasm_module
                ),
                span: Some(binding.span.clone()),
            });
        }

        if let Type::Arrow(params, ret, effects) = &binding.typ {
            lowered_externals.push(AnfExternal {
                name: binding.name.clone(),
                wasm_module,
                wasm_name: binding.wasm_name.clone(),
                params: params
                    .iter()
                    .map(|(n, t)| AnfParam {
                        label: n.clone(),
                        name: n.clone(), // Sigil handled later or not needed for externals?
                        typ: t.clone(),
                    })
                    .collect(),
                ret_type: *ret.clone(),
                effects: *effects.clone(),
            });
        }
    }

    Ok(AnfProgram {
        functions: lowered,
        externals: lowered_externals,
    })
}

fn collect_all_definitions(program: &Program) -> Result<CollectedDefinitions, LowerError> {
    let mut all_definitions = Vec::new();
    let mut external_bindings = HashMap::new();
    if let Ok(stdlib_programs) = load_stdlib_nx_programs() {
        for (_, stdlib_program) in stdlib_programs {
            collect_program_sum_type_defs_only(&stdlib_program, &mut all_definitions);
            collect_program_externals_only(
                &stdlib_program,
                &mut all_definitions,
                &mut external_bindings,
            );
        }
    }
    collect_program_definitions(program, &mut all_definitions, &mut external_bindings)?;
    Ok(CollectedDefinitions {
        definitions: all_definitions,
        external_bindings,
    })
}

fn collect_program_sum_type_defs_only(
    program: &Program,
    definitions_out: &mut Vec<Spanned<TopLevel>>,
) {
    for def in &program.definitions {
        if matches!(&def.node, TopLevel::Enum(_) | TopLevel::Exception(_)) {
            definitions_out.push(def.clone());
        }
    }
}

fn collect_program_externals_only(
    program: &Program,
    definitions_out: &mut Vec<Spanned<TopLevel>>,
    externals_out: &mut HashMap<String, ExternalBinding>,
) {
    let mut current_external_module: Option<String> = None;
    for def in &program.definitions {
        match &def.node {
            TopLevel::Import(import) if import.is_external => {
                current_external_module = Some(import.path.clone());
                definitions_out.push(def.clone());
            }
            TopLevel::Let(gl) => {
                if let Expr::External(wasm_name, typ) = &gl.value.node {
                    externals_out.insert(
                        gl.name.clone(),
                        ExternalBinding {
                            name: gl.name.clone(),
                            wasm_name: wasm_name.clone(),
                            typ: typ.clone(),
                            wasm_module: current_external_module.clone(),
                            span: def.span.clone(),
                        },
                    );
                    definitions_out.push(def.clone());
                }
            }
            _ => {}
        }
    }
}

fn collect_program_definitions(
    program: &Program,
    definitions_out: &mut Vec<Spanned<TopLevel>>,
    externals_out: &mut HashMap<String, ExternalBinding>,
) -> Result<(), LowerError> {
    let mut import_stack = Vec::new();
    collect_program_definitions_inner(program, definitions_out, externals_out, &mut import_stack)
}

fn collect_program_definitions_inner(
    program: &Program,
    definitions_out: &mut Vec<Spanned<TopLevel>>,
    externals_out: &mut HashMap<String, ExternalBinding>,
    import_stack: &mut Vec<String>,
) -> Result<(), LowerError> {
    let mut current_external_module: Option<String> = None;
    for def in &program.definitions {
        match &def.node {
            TopLevel::Import(import) if import.is_external => {
                current_external_module = Some(import.path.clone());
                definitions_out.push(def.clone());
            }
            TopLevel::Import(import) => {
                if import_stack.iter().any(|p| p == &import.path) {
                    return Err(LowerError {
                        message: format!("cyclic module import detected at '{}'", import.path),
                        span: Some(def.span.clone()),
                    });
                }
                import_stack.push(import.path.clone());
                let src = fs::read_to_string(&import.path).map_err(|e| LowerError {
                    message: format!("Failed to read {}: {}", import.path, e),
                    span: Some(def.span.clone()),
                })?;
                let imported_program = parser::parser().parse(src).map_err(|e| LowerError {
                    message: format!("Failed to parse {}: {:?}", import.path, e),
                    span: Some(def.span.clone()),
                })?;
                let rewritten = rewrite_imported_program(&imported_program, import, &def.span)?;
                let res = collect_program_definitions_inner(
                    &rewritten,
                    definitions_out,
                    externals_out,
                    import_stack,
                );
                import_stack.pop();
                res?;
            }
            TopLevel::Let(gl) => {
                if let Expr::External(wasm_name, typ) = &gl.value.node {
                    externals_out.insert(
                        gl.name.clone(),
                        ExternalBinding {
                            name: gl.name.clone(),
                            wasm_name: wasm_name.clone(),
                            typ: typ.clone(),
                            wasm_module: current_external_module.clone(),
                            span: def.span.clone(),
                        },
                    );
                }
                definitions_out.push(def.clone());
            }
            _ => {
                definitions_out.push(def.clone());
            }
        }
    }
    Ok(())
}

fn rewrite_imported_program(
    program: &Program,
    import: &Import,
    import_span: &Span,
) -> Result<Program, LowerError> {
    let rename_map = build_import_rename_map(program, import, import_span)?;
    let definitions = program
        .definitions
        .iter()
        .map(|def| rewrite_top_level_calls(def, &rename_map))
        .collect();
    Ok(Program { definitions })
}

fn build_import_rename_map(
    program: &Program,
    import: &Import,
    import_span: &Span,
) -> Result<HashMap<String, String>, LowerError> {
    let mut map = HashMap::new();
    if import.items.is_empty() {
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

    let mut selected = HashSet::new();
    for item in &import.items {
        let found_public = program
            .definitions
            .iter()
            .any(|def| matches!(&def.node, TopLevel::Let(gl) if gl.is_public && gl.name == *item));
        if !found_public {
            return Err(LowerError {
                message: format!("Item {} not found in {}", item, import.path),
                span: Some(import_span.clone()),
            });
        }
        selected.insert(item.clone());
    }

    let hidden_prefix = format!(
        "__import_{}_{}",
        get_default_alias(&import.path),
        import_span.start
    );
    for def in &program.definitions {
        if let TopLevel::Let(gl) = &def.node {
            let renamed = if gl.is_public && selected.contains(&gl.name) {
                gl.name.clone()
            } else {
                format!("{}.{}", hidden_prefix, gl.name)
            };
            map.insert(gl.name.clone(), renamed);
        }
    }
    Ok(map)
}

fn rewrite_top_level_calls(
    def: &Spanned<TopLevel>,
    rename_map: &HashMap<String, String>,
) -> Spanned<TopLevel> {
    match &def.node {
        TopLevel::Let(gl) => {
            let mut next = gl.clone();
            if let Some(renamed) = rename_map.get(&next.name) {
                next.name = renamed.clone();
            }
            next.value = rewrite_expr_calls(&next.value, rename_map);
            Spanned {
                node: TopLevel::Let(next),
                span: def.span.clone(),
            }
        }
        _ => def.clone(),
    }
}

fn rewrite_expr_calls(expr: &Spanned<Expr>, rename_map: &HashMap<String, String>) -> Spanned<Expr> {
    let node = match &expr.node {
        Expr::Literal(_) | Expr::Variable(_, _) | Expr::Borrow(_, _) | Expr::External(_, _) => {
            expr.node.clone()
        }
        Expr::BinaryOp(lhs, op, rhs) => Expr::BinaryOp(
            Box::new(rewrite_expr_calls(lhs, rename_map)),
            op.clone(),
            Box::new(rewrite_expr_calls(rhs, rename_map)),
        ),
        Expr::Call {
            func,
            args,
            perform,
        } => {
            let next_func = rename_map
                .get(func)
                .cloned()
                .unwrap_or_else(|| func.clone());
            let next_args = args
                .iter()
                .map(|(label, value)| (label.clone(), rewrite_expr_calls(value, rename_map)))
                .collect();
            Expr::Call {
                func: next_func,
                args: next_args,
                perform: *perform,
            }
        }
        Expr::Constructor(name, args) => Expr::Constructor(
            name.clone(),
            args.iter()
                .map(|(label, value)| (label.clone(), rewrite_expr_calls(value, rename_map)))
                .collect(),
        ),
        Expr::Record(fields) => Expr::Record(
            fields
                .iter()
                .map(|(label, value)| (label.clone(), rewrite_expr_calls(value, rename_map)))
                .collect(),
        ),
        Expr::Array(values) => Expr::Array(
            values
                .iter()
                .map(|value| rewrite_expr_calls(value, rename_map))
                .collect(),
        ),
        Expr::Index(lhs, rhs) => Expr::Index(
            Box::new(rewrite_expr_calls(lhs, rename_map)),
            Box::new(rewrite_expr_calls(rhs, rename_map)),
        ),
        Expr::FieldAccess(value, field) => Expr::FieldAccess(
            Box::new(rewrite_expr_calls(value, rename_map)),
            field.clone(),
        ),
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => Expr::If {
            cond: Box::new(rewrite_expr_calls(cond, rename_map)),
            then_branch: rewrite_stmts_calls(then_branch, rename_map),
            else_branch: else_branch
                .as_ref()
                .map(|branch| rewrite_stmts_calls(branch, rename_map)),
        },
        Expr::Match { target, cases } => Expr::Match {
            target: Box::new(rewrite_expr_calls(target, rename_map)),
            cases: cases
                .iter()
                .map(|case| rewrite_match_case_calls(case, rename_map))
                .collect(),
        },
        Expr::Lambda {
            type_params,
            params,
            ret_type,
            effects,
            body,
        } => Expr::Lambda {
            type_params: type_params.clone(),
            params: params.clone(),
            ret_type: ret_type.clone(),
            effects: effects.clone(),
            body: rewrite_stmts_calls(body, rename_map),
        },
        Expr::Raise(value) => Expr::Raise(Box::new(rewrite_expr_calls(value, rename_map))),
    };

    Spanned {
        node,
        span: expr.span.clone(),
    }
}

fn rewrite_match_case_calls(case: &MatchCase, rename_map: &HashMap<String, String>) -> MatchCase {
    MatchCase {
        pattern: case.pattern.clone(),
        body: rewrite_stmts_calls(&case.body, rename_map),
    }
}

fn rewrite_stmts_calls(
    stmts: &[Spanned<Stmt>],
    rename_map: &HashMap<String, String>,
) -> Vec<Spanned<Stmt>> {
    stmts
        .iter()
        .map(|stmt| rewrite_stmt_calls(stmt, rename_map))
        .collect()
}

fn rewrite_stmt_calls(stmt: &Spanned<Stmt>, rename_map: &HashMap<String, String>) -> Spanned<Stmt> {
    let node = match &stmt.node {
        Stmt::Let {
            name,
            sigil,
            typ,
            value,
        } => Stmt::Let {
            name: name.clone(),
            sigil: sigil.clone(),
            typ: typ.clone(),
            value: rewrite_expr_calls(value, rename_map),
        },
        Stmt::Drop(value) => Stmt::Drop(rewrite_expr_calls(value, rename_map)),
        Stmt::Expr(value) => Stmt::Expr(rewrite_expr_calls(value, rename_map)),
        Stmt::Return(value) => Stmt::Return(rewrite_expr_calls(value, rename_map)),
        Stmt::Assign { target, value } => Stmt::Assign {
            target: rewrite_expr_calls(target, rename_map),
            value: rewrite_expr_calls(value, rename_map),
        },
        Stmt::Conc(tasks) => Stmt::Conc(
            tasks
                .iter()
                .map(|task| {
                    let mut next = task.clone();
                    next.body = rewrite_stmts_calls(&task.body, rename_map);
                    next
                })
                .collect(),
        ),
        Stmt::Try {
            body,
            catch_param,
            catch_body,
        } => Stmt::Try {
            body: rewrite_stmts_calls(body, rename_map),
            catch_param: catch_param.clone(),
            catch_body: rewrite_stmts_calls(catch_body, rename_map),
        },
        Stmt::Comment => Stmt::Comment,
    };

    Spanned {
        node,
        span: stmt.span.clone(),
    }
}

fn get_default_alias(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

/// Collects all functions transitively reachable from `main`.
pub fn collect_reachable_functions(
    functions: &HashMap<String, Function>,
    externals: &HashMap<String, ExternalBinding>,
) -> HashSet<String> {
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::new();
    if functions.contains_key("main") {
        queue.push_back("main".to_string());
    }

    while let Some(name) = queue.pop_front() {
        if !reachable.insert(name.clone()) {
            continue;
        }
        if let Some(func) = functions.get(&name) {
            let mut calls = Vec::new();
            collect_calls_in_stmts(&func.body, &mut calls);
            for called in calls {
                if (functions.contains_key(&called) || externals.contains_key(&called))
                    && !reachable.contains(&called)
                {
                    queue.push_back(called);
                }
            }
        }
    }
    reachable
}

fn collect_calls_in_stmts(stmts: &[Spanned<Stmt>], out: &mut Vec<String>) {
    for s in stmts {
        match &s.node {
            Stmt::Let { value, .. } => collect_calls_in_expr(value, out),
            Stmt::Drop(e) | Stmt::Expr(e) | Stmt::Return(e) => collect_calls_in_expr(e, out),
            Stmt::Assign { target, value } => {
                collect_calls_in_expr(target, out);
                collect_calls_in_expr(value, out);
            }
            Stmt::Conc(tasks) => {
                for task in tasks {
                    collect_calls_in_stmts(&task.body, out);
                }
            }
            Stmt::Try {
                body, catch_body, ..
            } => {
                collect_calls_in_stmts(body, out);
                collect_calls_in_stmts(catch_body, out);
            }
            Stmt::Comment => {}
        }
    }
}

fn collect_calls_in_expr(expr: &Spanned<Expr>, out: &mut Vec<String>) {
    match &expr.node {
        Expr::Call { func, args, .. } => {
            out.push(func.clone());
            for (_, e) in args {
                collect_calls_in_expr(e, out);
            }
        }
        Expr::BinaryOp(l, _, r) | Expr::Index(l, r) => {
            collect_calls_in_expr(l, out);
            collect_calls_in_expr(r, out);
        }
        Expr::Constructor(_, args) => {
            for (_, e) in args {
                collect_calls_in_expr(e, out);
            }
        }
        Expr::Array(args) => {
            for e in args {
                collect_calls_in_expr(e, out);
            }
        }
        Expr::Record(fields) => {
            for (_, e) in fields {
                collect_calls_in_expr(e, out);
            }
        }
        Expr::FieldAccess(receiver, _) | Expr::Raise(receiver) => {
            collect_calls_in_expr(receiver, out)
        }
        Expr::If {
            cond,
            then_branch,
            else_branch,
        } => {
            collect_calls_in_expr(cond, out);
            collect_calls_in_stmts(then_branch, out);
            if let Some(else_branch) = else_branch {
                collect_calls_in_stmts(else_branch, out);
            }
        }
        Expr::Match { target, cases } => {
            collect_calls_in_expr(target, out);
            for case in cases {
                collect_calls_in_pattern(&case.pattern, out);
                collect_calls_in_stmts(&case.body, out);
            }
        }
        Expr::Lambda { body, .. } => collect_calls_in_stmts(body, out),
        Expr::Variable(name, _) => out.push(name.clone()),
        Expr::Literal(_) | Expr::Borrow(_, _) | Expr::External(_, _) => {}
    }
}

fn collect_calls_in_pattern(p: &Spanned<Pattern>, out: &mut Vec<String>) {
    match &p.node {
        Pattern::Constructor(_, args) => {
            for (_, p) in args {
                collect_calls_in_pattern(p, out);
            }
        }
        Pattern::Record(fields, _) => {
            for (_, p) in fields {
                collect_calls_in_pattern(p, out);
            }
        }
        Pattern::Literal(_) | Pattern::Variable(_, _) | Pattern::Wildcard => {
            let _ = out;
        }
    }
}

struct LowerCtx<'a> {
    signatures: &'a HashMap<String, Signature>,
    enums: &'a HashMap<String, EnumDef>,
    exceptions: &'a HashMap<String, ExceptionDef>,
    vars: HashMap<String, Type>,
    stmts: Vec<AnfStmt>,
    temp_counter: usize,
    current_ret_type: Type,
}

impl<'a> LowerCtx<'a> {
    fn new(
        signatures: &'a HashMap<String, Signature>,
        enums: &'a HashMap<String, EnumDef>,
        exceptions: &'a HashMap<String, ExceptionDef>,
    ) -> Self {
        Self {
            signatures,
            enums,
            exceptions,
            vars: HashMap::new(),
            stmts: Vec::new(),
            temp_counter: 0,
            current_ret_type: Type::Unit,
        }
    }

    fn lower_function(&mut self, func: &Function) -> Result<AnfFunction, LowerError> {
        self.vars.clear();
        self.stmts.clear();
        self.temp_counter = 0;
        self.current_ret_type = func.ret_type.clone();

        let params = func
            .params
            .iter()
            .map(|p| {
                let key = p.sigil.get_key(&p.name);
                self.vars.insert(key.clone(), p.typ.clone());
                AnfParam {
                    label: p.name.clone(),
                    name: key,
                    typ: p.typ.clone(),
                }
            })
            .collect::<Vec<_>>();

        let mut ret_atom: Option<AnfAtom> = None;
        for stmt in &func.body {
            if ret_atom.is_some() {
                break;
            }
            if let Some(ret) = self.lower_stmt(stmt)? {
                ret_atom = Some(ret);
            }
        }

        let ret = if let Some(ret) = ret_atom {
            ret
        } else if let Some(ret) = fallback_return_atom_from_terminal_stmt(&self.stmts) {
            ret
        } else if matches!(func.ret_type, Type::Unit) {
            AnfAtom::Unit
        } else {
            return Err(LowerError {
                message: format!("function '{}' may not return a value", func.name),
                span: None,
            });
        };

        Ok(AnfFunction {
            name: func.name.clone(),
            params,
            ret_type: func.ret_type.clone(),
            effects: func.effects.clone(),
            body: self.stmts.clone(),
            ret,
        })
    }

    fn lower_stmt(&mut self, stmt: &Spanned<Stmt>) -> Result<Option<AnfAtom>, LowerError> {
        match &stmt.node {
            Stmt::Let {
                name,
                sigil,
                typ,
                value,
            } => {
                let atom = self.lower_expr_to_atom(value)?;
                let inferred = atom.typ();
                let final_type = typ.clone().unwrap_or(inferred);
                let key = sigil.get_key(name);
                self.stmts.push(AnfStmt::Let {
                    name: key.clone(),
                    typ: final_type.clone(),
                    expr: AnfExpr::Atom(atom),
                });
                self.vars.insert(key, final_type);
                Ok(None)
            }
            Stmt::Drop(expr) => {
                let atom = self.lower_expr_to_atom(expr)?;
                self.stmts.push(AnfStmt::Drop(atom));
                Ok(None)
            }
            Stmt::Expr(expr) => {
                if let Expr::If {
                    cond,
                    then_branch,
                    else_branch,
                } = &expr.node
                {
                    let cond_atom = self.lower_expr_to_atom(cond)?;
                    let (then_body, then_ret) = self.lower_block(then_branch, self.vars.clone())?;
                    let then_ret =
                        then_ret.or_else(|| fallback_return_atom_from_terminal_stmt(&then_body));
                    let (else_body, else_ret) = if let Some(else_branch) = else_branch {
                        let (body, ret) = self.lower_block(else_branch, self.vars.clone())?;
                        let ret = ret.or_else(|| fallback_return_atom_from_terminal_stmt(&body));
                        (body, ret)
                    } else {
                        (Vec::new(), None)
                    };
                    if then_ret.is_none() && else_ret.is_none() {
                        self.stmts.push(AnfStmt::If {
                            cond: cond_atom,
                            then_body,
                            else_body,
                        });
                    } else {
                        let then_ret = then_ret.ok_or_else(|| LowerError {
                            message: "if branch must return a value in current wasm ANF lowering"
                                .to_string(),
                            span: Some(expr.span.clone()),
                        })?;
                        self.stmts.push(AnfStmt::IfReturn {
                            cond: cond_atom,
                            then_body,
                            then_ret,
                            else_body,
                            else_ret,
                            ret_type: self.current_ret_type.clone(),
                        });
                    }
                } else if let Expr::Match { target, cases } = &expr.node {
                    let target_atom = self.lower_expr_to_atom(target)?;
                    let mut chain: Option<AnfStmt> = None;
                    for case in cases.iter().rev() {
                        let (cond, then_body, then_ret) =
                            self.lower_match_case(&target_atom, &case.pattern, &case.body)?;
                        let else_body = chain.take().map_or_else(Vec::new, |next| vec![next]);
                        let cond = if else_body.is_empty() {
                            // Last remaining arm: treat as fallback after typechecked exhaustiveness.
                            AnfAtom::Bool(true)
                        } else {
                            cond.unwrap_or(AnfAtom::Bool(true))
                        };
                        chain = Some(AnfStmt::IfReturn {
                            cond,
                            then_body,
                            then_ret,
                            else_body,
                            else_ret: None,
                            ret_type: self.current_ret_type.clone(),
                        });
                    }
                    if let Some(stmt) = chain {
                        self.stmts.push(stmt);
                    }
                } else {
                    let _ = self.lower_expr_to_atom(expr)?;
                }
                Ok(None)
            }
            Stmt::Return(expr) => {
                let atom = self.lower_expr_to_atom(expr)?;
                Ok(Some(atom))
            }
            Stmt::Try {
                body,
                catch_param,
                catch_body,
            } => {
                let scope_vars = self.vars.clone();
                let (body_stmts, body_ret) = self.lower_block(body, scope_vars.clone())?;

                let mut catch_vars = scope_vars;
                let exn_type = Type::UserDefined("Exn".to_string(), vec![]);
                catch_vars.insert(catch_param.clone(), exn_type.clone());
                let (catch_stmts, catch_ret) = self.lower_block(catch_body, catch_vars)?;

                self.stmts.push(AnfStmt::TryCatch {
                    body: body_stmts,
                    body_ret,
                    catch_param: catch_param.clone(),
                    catch_param_typ: exn_type,
                    catch_body: catch_stmts,
                    catch_ret,
                });
                Ok(None)
            }
            Stmt::Comment => Ok(None),
            Stmt::Assign { .. } | Stmt::Conc(_) => Err(LowerError {
                message: "statement is not supported by current wasm ANF lowering".to_string(),
                span: Some(stmt.span.clone()),
            }),
        }
    }

    fn lower_block(
        &mut self,
        block: &[Spanned<Stmt>],
        vars: HashMap<String, Type>,
    ) -> Result<(Vec<AnfStmt>, Option<AnfAtom>), LowerError> {
        let saved_vars = std::mem::replace(&mut self.vars, vars);
        let saved_stmts = std::mem::take(&mut self.stmts);

        let mut ret_atom = None;
        for stmt in block {
            if ret_atom.is_some() {
                break;
            }
            if let Some(ret) = self.lower_stmt(stmt)? {
                ret_atom = Some(ret);
            }
        }

        let lowered = std::mem::take(&mut self.stmts);
        self.stmts = saved_stmts;
        self.vars = saved_vars;
        Ok((lowered, ret_atom))
    }

    fn lower_expr_to_atom(&mut self, expr: &Spanned<Expr>) -> Result<AnfAtom, LowerError> {
        match &expr.node {
            Expr::Literal(lit) => Ok(match lit {
                Literal::Int(i) => AnfAtom::Int(*i),
                Literal::Float(f) => AnfAtom::Float(*f),
                Literal::Bool(b) => AnfAtom::Bool(*b),
                Literal::String(s) => AnfAtom::String(s.clone()),
                Literal::Unit => AnfAtom::Unit,
            }),
            Expr::Variable(name, sigil) => {
                let key = sigil_key(name, sigil);
                if let Some(typ) = self.vars.get(&key).cloned() {
                    Ok(AnfAtom::Var { name: key, typ })
                } else if let Some(sig) = self.signatures.get(&key) {
                    Ok(AnfAtom::Var {
                        name: key,
                        typ: signature_type(sig),
                    })
                } else if let Some(sig) = self.signatures.get(name) {
                    Ok(AnfAtom::Var {
                        name: name.clone(),
                        typ: signature_type(sig),
                    })
                } else {
                    Err(LowerError {
                        message: format!("unknown variable '{}'", key),
                        span: Some(expr.span.clone()),
                    })
                }
            }
            Expr::Borrow(name, sigil) => {
                let key = sigil_key(name, sigil);
                let typ = self.vars.get(&key).cloned().ok_or_else(|| LowerError {
                    message: format!("unknown variable '{}'", key),
                    span: Some(expr.span.clone()),
                })?;
                let inner = match typ {
                    Type::Linear(i) | Type::Borrow(i) => *i,
                    other => other,
                };
                Ok(AnfAtom::Var {
                    name: key,
                    typ: Type::Borrow(Box::new(inner)),
                })
            }
            Expr::BinaryOp(lhs, op, rhs) => {
                let lhs = self.lower_expr_to_atom(lhs)?;
                let rhs = self.lower_expr_to_atom(rhs)?;
                let typ = infer_binary_type(op, &lhs.typ(), &rhs.typ());
                self.bind_expr_to_temp(
                    AnfExpr::Binary {
                        op: op.clone(),
                        lhs,
                        rhs,
                        typ: typ.clone(),
                    },
                    typ,
                )
            }
            Expr::Call {
                func,
                args,
                perform,
            } => {
                let sig = self.signatures.get(func).ok_or_else(|| LowerError {
                    message: format!("unknown function '{}'", func),
                    span: Some(expr.span.clone()),
                })?;
                if sig.is_generic {
                    return Err(LowerError {
                        message: format!(
                            "call to generic function '{}' is not supported by current wasm ANF lowering",
                            func
                        ),
                        span: Some(expr.span.clone()),
                    });
                }
                let mut lowered_args = Vec::with_capacity(args.len());
                for (label, arg_expr) in args {
                    lowered_args.push((label.clone(), self.lower_expr_to_atom(arg_expr)?));
                }
                self.bind_expr_to_temp(
                    AnfExpr::Call {
                        func: func.clone(),
                        args: lowered_args,
                        typ: sig.ret.clone(),
                        perform: *perform,
                    },
                    sig.ret.clone(),
                )
            }
            Expr::Constructor(name, args) => {
                let fields = self
                    .lookup_constructor_fields(name)
                    .ok_or_else(|| LowerError {
                        message: format!("unknown constructor '{}'", name),
                        span: Some(expr.span.clone()),
                    })?;

                let mut lowered_args = Vec::with_capacity(fields.len());
                for (idx, m) in match_args_in_decl_order(args, &fields)
                    .into_iter()
                    .enumerate()
                {
                    let arg = m.ok_or_else(|| LowerError {
                        message: format!(
                            "missing constructor field at position {} for '{}'",
                            idx, name
                        ),
                        span: Some(expr.span.clone()),
                    })?;
                    lowered_args.push(self.lower_expr_to_atom(arg)?);
                }

                let typ = self.lookup_constructor_type(name, &expr.span)?;

                self.bind_expr_to_temp(
                    AnfExpr::Constructor {
                        name: name.clone(),
                        args: lowered_args,
                        typ: typ.clone(),
                    },
                    typ,
                )
            }
            Expr::Record(fields) => {
                let mut lowered_fields = Vec::with_capacity(fields.len());
                for (label, value) in fields {
                    lowered_fields.push((label.clone(), self.lower_expr_to_atom(value)?));
                }
                lowered_fields.sort_by(|a, b| a.0.cmp(&b.0));

                let typ = Type::Record(
                    lowered_fields
                        .iter()
                        .map(|(label, atom)| (label.clone(), atom.typ()))
                        .collect(),
                );

                self.bind_expr_to_temp(
                    AnfExpr::Record {
                        fields: lowered_fields,
                        typ: typ.clone(),
                    },
                    typ,
                )
            }
            Expr::Raise(value) => {
                let atom = self.lower_expr_to_atom(value)?;
                self.bind_expr_to_temp(
                    AnfExpr::Raise {
                        value: atom,
                        typ: Type::Unit,
                    },
                    Type::Unit,
                )
            }
            Expr::Array(_)
            | Expr::Index(_, _)
            | Expr::FieldAccess(_, _)
            | Expr::If { .. }
            | Expr::Match { .. }
            | Expr::Lambda { .. }
            | Expr::External(_, _) => Err(LowerError {
                message: "expression is not supported by current wasm ANF lowering".to_string(),
                span: Some(expr.span.clone()),
            }),
        }
    }

    fn lower_match_case(
        &mut self,
        target: &AnfAtom,
        pattern: &Spanned<Pattern>,
        branch: &[Spanned<Stmt>],
    ) -> Result<(Option<AnfAtom>, Vec<AnfStmt>, AnfAtom), LowerError> {
        let (cond, bindings) = self.lower_match_case_condition_and_bindings(target, pattern)?;
        let mut branch_vars = self.vars.clone();
        let mut binding_entries: Vec<(String, AnfAtom)> = bindings.into_iter().collect();
        binding_entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, atom) in &binding_entries {
            branch_vars.insert(name.clone(), atom.typ());
        }

        let (mut then_body, then_ret) = self.lower_block(branch, branch_vars)?;
        let then_ret = then_ret
            .or_else(|| fallback_return_atom_from_terminal_stmt(&then_body))
            .ok_or_else(|| LowerError {
                message: "match case must return a value in current wasm ANF lowering".to_string(),
                span: Some(pattern.span.clone()),
            })?;

        let mut binding_stmts = Vec::with_capacity(binding_entries.len());
        for (name, atom) in binding_entries {
            let typ = atom.typ();
            binding_stmts.push(AnfStmt::Let {
                name,
                typ,
                expr: AnfExpr::Atom(atom),
            });
        }
        binding_stmts.append(&mut then_body);
        Ok((cond, binding_stmts, then_ret))
    }

    fn lower_match_case_condition_and_bindings(
        &mut self,
        target: &AnfAtom,
        pattern: &Spanned<Pattern>,
    ) -> Result<(Option<AnfAtom>, HashMap<String, AnfAtom>), LowerError> {
        let mut conds = Vec::new();
        let mut bindings = HashMap::new();
        self.collect_pattern_conditions_and_bindings(
            target,
            &target.typ(),
            pattern,
            &mut conds,
            &mut bindings,
        )?;
        let cond = self.combine_bool_conditions(conds)?;
        Ok((cond, bindings))
    }

    fn collect_pattern_conditions_and_bindings(
        &mut self,
        target: &AnfAtom,
        target_type: &Type,
        pattern: &Spanned<Pattern>,
        conds: &mut Vec<AnfAtom>,
        bindings: &mut HashMap<String, AnfAtom>,
    ) -> Result<(), LowerError> {
        match &pattern.node {
            Pattern::Wildcard => Ok(()),
            Pattern::Variable(name, sigil) => {
                bindings.insert(sigil_key(name, sigil), target.clone());
                Ok(())
            }
            Pattern::Literal(lit) => {
                if let Some(cond) =
                    self.build_literal_condition(target, target_type, lit, &pattern.span)?
                {
                    conds.push(cond);
                }
                Ok(())
            }
            Pattern::Constructor(name, args) => {
                let fields = self
                    .lookup_constructor_fields(name)
                    .ok_or_else(|| LowerError {
                        message: format!("unknown constructor '{}'", name),
                        span: Some(pattern.span.clone()),
                    })?;

                let tag_atom = self.bind_expr_to_temp(
                    AnfExpr::ObjectTag {
                        value: target.clone(),
                        typ: Type::I64,
                    },
                    Type::I64,
                )?;
                let expected_tag = AnfAtom::Int(constructor_tag(name, fields.len()));
                let tag_cond = self.bind_expr_to_temp(
                    AnfExpr::Binary {
                        op: "==".to_string(),
                        lhs: tag_atom,
                        rhs: expected_tag,
                        typ: Type::Bool,
                    },
                    Type::Bool,
                )?;
                conds.push(tag_cond);

                let ordered_pats = match_args_in_decl_order(args, &fields);
                for (idx, pat_opt) in ordered_pats.into_iter().enumerate() {
                    if let Some(pat) = pat_opt {
                        let field_type = fields[idx].1.clone();
                        let field_atom = self.bind_expr_to_temp(
                            AnfExpr::ObjectField {
                                value: target.clone(),
                                index: idx,
                                typ: field_type.clone(),
                            },
                            field_type.clone(),
                        )?;
                        self.collect_pattern_conditions_and_bindings(
                            &field_atom,
                            &field_type,
                            pat,
                            conds,
                            bindings,
                        )?;
                    }
                }
                Ok(())
            }
            Pattern::Record(pattern_fields, open) => {
                let mut target_fields = match peel_linear(target_type) {
                    Type::Record(fields) => fields.clone(),
                    other => {
                        return Err(LowerError {
                            message: format!(
                                "record pattern target must be a record, got '{}'",
                                other
                            ),
                            span: Some(pattern.span.clone()),
                        });
                    }
                };
                target_fields.sort_by(|a, b| a.0.cmp(&b.0));

                if !*open && pattern_fields.len() != target_fields.len() {
                    return Err(LowerError {
                        message: "closed record pattern must list all fields".to_string(),
                        span: Some(pattern.span.clone()),
                    });
                }

                for (label, pat) in pattern_fields {
                    let (idx, (_, field_type)) = target_fields
                        .iter()
                        .enumerate()
                        .find(|(_, f)| f.0 == *label)
                        .ok_or_else(|| LowerError {
                            message: format!("unknown record field '{}' in pattern", label),
                            span: Some(pattern.span.clone()),
                        })?;
                    let field_typ = field_type.clone();
                    let field_atom = self.bind_expr_to_temp(
                        AnfExpr::ObjectField {
                            value: target.clone(),
                            index: idx,
                            typ: field_typ.clone(),
                        },
                        field_typ.clone(),
                    )?;
                    self.collect_pattern_conditions_and_bindings(
                        &field_atom,
                        &field_typ,
                        pat,
                        conds,
                        bindings,
                    )?;
                }
                Ok(())
            }
        }
    }

    fn bind_expr_to_temp(&mut self, expr: AnfExpr, typ: Type) -> Result<AnfAtom, LowerError> {
        let name = self.new_temp();
        self.stmts.push(AnfStmt::Let {
            name: name.clone(),
            typ: typ.clone(),
            expr,
        });
        self.vars.insert(name.clone(), typ.clone());
        Ok(AnfAtom::Var { name, typ })
    }

    fn new_temp(&mut self) -> String {
        let name = format!("__t{}", self.temp_counter);
        self.temp_counter += 1;
        name
    }

    fn build_literal_condition(
        &mut self,
        target: &AnfAtom,
        target_type: &Type,
        lit: &Literal,
        span: &Span,
    ) -> Result<Option<AnfAtom>, LowerError> {
        let rhs = match lit {
            Literal::Int(i) => AnfAtom::Int(*i),
            Literal::Float(f) => AnfAtom::Float(*f),
            Literal::Bool(b) => AnfAtom::Bool(*b),
            Literal::Unit => return Ok(None),
            Literal::String(_) => {
                return Err(LowerError {
                    message:
                        "string literal match patterns are not supported by current wasm ANF lowering"
                            .to_string(),
                    span: Some(span.clone()),
                });
            }
        };

        let op = if matches!(peel_linear(target_type), Type::F32 | Type::F64) {
            "==.".to_string()
        } else {
            "==".to_string()
        };

        self.bind_expr_to_temp(
            AnfExpr::Binary {
                op,
                lhs: target.clone(),
                rhs,
                typ: Type::Bool,
            },
            Type::Bool,
        )
        .map(Some)
    }

    fn combine_bool_conditions(
        &mut self,
        conds: Vec<AnfAtom>,
    ) -> Result<Option<AnfAtom>, LowerError> {
        let mut iter = conds.into_iter();
        let Some(mut current) = iter.next() else {
            return Ok(None);
        };
        for cond in iter {
            current = self.bind_expr_to_temp(
                AnfExpr::Binary {
                    op: "&&".to_string(),
                    lhs: current,
                    rhs: cond,
                    typ: Type::Bool,
                },
                Type::Bool,
            )?;
        }
        Ok(Some(current))
    }

    fn lookup_constructor_fields(&self, ctor: &str) -> Option<Vec<(Option<String>, Type)>> {
        for ed in self.enums.values() {
            if let Some(v) = ed.variants.iter().find(|v| v.name == ctor) {
                return Some(v.fields.clone());
            }
        }
        self.exceptions.get(ctor).map(|ex| ex.fields.clone())
    }

    fn lookup_exception_type(&self, ctor: &str) -> Option<Type> {
        if self.exceptions.contains_key(ctor) {
            Some(Type::UserDefined("Exn".to_string(), vec![]))
        } else {
            None
        }
    }

    fn lookup_constructor_type(&self, ctor: &str, span: &Span) -> Result<Type, LowerError> {
        for ed in self.enums.values() {
            if ed.type_params.is_empty() && ed.variants.iter().any(|v| v.name == ctor) {
                return Ok(Type::UserDefined(ed.name.clone(), vec![]));
            }
            if !ed.type_params.is_empty() && ed.variants.iter().any(|v| v.name == ctor) {
                let type_args = ed
                    .type_params
                    .iter()
                    .map(|p| Type::Var(format!("__{}@{}", p, ctor)))
                    .collect();
                return Ok(Type::UserDefined(ed.name.clone(), type_args));
            }
        }
        if let Some(exn_type) = self.lookup_exception_type(ctor) {
            return Ok(exn_type);
        }
        Err(LowerError {
            message: format!("unknown constructor '{}'", ctor),
            span: Some(span.clone()),
        })
    }
}

fn fallback_return_atom_from_terminal_stmt(stmts: &[AnfStmt]) -> Option<AnfAtom> {
    match stmts.last() {
        Some(AnfStmt::IfReturn { then_ret, .. }) => Some(then_ret.clone()),
        Some(AnfStmt::TryCatch {
            body_ret,
            catch_ret,
            ..
        }) => catch_ret.clone().or_else(|| body_ret.clone()),
        _ => None,
    }
}

fn match_args_in_decl_order<'a, T>(
    args: &'a [(Option<String>, T)],
    fields: &[(Option<String>, Type)],
) -> Vec<Option<&'a T>> {
    let mut matched = vec![None; fields.len()];
    for (label, item) in args {
        if let Some(l) = label {
            if let Some(idx) = fields.iter().position(|f| f.0.as_ref() == Some(l)) {
                matched[idx] = Some(item);
            }
        } else if let Some(idx) = matched.iter().position(|m| m.is_none()) {
            matched[idx] = Some(item);
        }
    }
    matched
}

fn constructor_tag(name: &str, arity: usize) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in name.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= arity as u64;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash as i64
}

fn signature_type(sig: &Signature) -> Type {
    Type::Arrow(
        sig.params.clone(),
        Box::new(sig.ret.clone()),
        Box::new(sig.effects.clone()),
    )
}

fn sigil_key(name: &str, sigil: &Sigil) -> String {
    sigil.get_key(name)
}

fn peel_linear(mut typ: &Type) -> &Type {
    while let Type::Linear(inner) = typ {
        typ = inner;
    }
    typ
}

fn infer_binary_type(op: &str, lhs: &Type, rhs: &Type) -> Type {
    if matches!(
        op,
        "==" | "!=" | "<" | "<=" | ">" | ">=" | "==." | "!=." | "<." | "<=." | ">." | ">=."
    ) {
        return Type::Bool;
    }
    if op == "++" || (op == "+" && matches!(lhs, Type::String) && matches!(rhs, Type::String)) {
        return Type::String;
    }
    if op.ends_with('.')
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
