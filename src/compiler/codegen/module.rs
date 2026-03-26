use std::collections::{HashMap, HashSet};

use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, ElementSection, Elements, EntityType, ExportKind,
    ExportSection, Function, FunctionSection, GlobalSection, GlobalType, ImportSection,
    Instruction, MemorySection, MemoryType, Module, NameMap, NameSection, RefType, TableSection,
    TableType, TagKind, TagSection, TagType, TypeSection, ValType,
};

use crate::constants::{ENTRYPOINT, MEMORY_EXPORT, WASI_CLI_RUN_EXPORT};
use crate::intern::Symbol;
use crate::ir::lir::{LirExpr, LirExternal, LirProgram, LirStmt};
use crate::types::Type;

use super::emit::{
    external_param_types, external_return_types, peel_linear, return_type_to_wasm_result,
    type_to_wasm_valtype,
};
use super::error::CodegenError;
use super::function::compile_function;
use super::layout::{build_codegen_layout, program_uses_object_heap, MemoryMode};
use super::{ALLOCATE_WASM_NAME, CONC_JOIN_NAME, CONC_MODULE, CONC_SPAWN_NAME, CONC_TASK_PREFIX};

/// Compiles LIR (in ANF) directly into core WASM bytes.
pub fn compile_lir_to_wasm(program: &LirProgram) -> Result<Vec<u8>, CodegenError> {
    let has_conc = program
        .functions
        .iter()
        .any(|f| f.name.starts_with(CONC_TASK_PREFIX));
    let has_eh = program_needs_eh(program);
    let n_conc_imports: u32 = if has_conc { 2 } else { 0 };

    let stdlib_alloc_module = if program_uses_object_heap(program) {
        program
            .externals
            .iter()
            .find(|ext| ext.wasm_module.ends_with("stdlib.wasm"))
            .map(|ext| ext.wasm_module.to_string())
    } else {
        None
    };
    let n_alloc_imports: u32 = if stdlib_alloc_module.is_some() { 1 } else { 0 };

    // Deduplicate externals by (wasm_module, wasm_name) — multiple Nexus names
    // pointing to the same underlying WASM function share a single WASM import.
    let mut wasm_import_dedup: HashMap<(Symbol, Symbol), u32> = HashMap::new();
    let mut deduped_externals: Vec<&LirExternal> = Vec::new();
    let mut external_function_indices = HashMap::new();
    for ext in &program.externals {
        let key = (ext.wasm_module, ext.wasm_name);
        if let Some(&idx) = wasm_import_dedup.get(&key) {
            // Reuse existing import index for this Nexus name
            external_function_indices.insert(ext.name, idx);
        } else {
            let idx = deduped_externals.len() as u32;
            wasm_import_dedup.insert(key, idx);
            deduped_externals.push(ext);
            external_function_indices.insert(ext.name, idx);
        }
    }
    let deduped_ext_count = deduped_externals.len() as u32;

    let import_count = deduped_ext_count + n_conc_imports + n_alloc_imports;

    let mut internal_function_indices = HashMap::new();
    for (idx, func) in program.functions.iter().enumerate() {
        let fidx = import_count + idx as u32;
        internal_function_indices.insert(func.name.clone(), fidx);
    }
    let entrypoint_sym = Symbol::from(ENTRYPOINT);
    let main_idx = internal_function_indices
        .get(&entrypoint_sym)
        .copied()
        .ok_or_else(|| CodegenError::MissingMain)?;
    let main_func = program
        .functions
        .iter()
        .find(|func| func.name == ENTRYPOINT)
        .ok_or_else(|| CodegenError::MissingMain)?;

    if has_conc {
        external_function_indices.insert(Symbol::from(CONC_SPAWN_NAME), deduped_ext_count);
        external_function_indices.insert(Symbol::from(CONC_JOIN_NAME), deduped_ext_count + 1);
    }

    // Collect funcref targets and indirect call types
    let funcref_targets = collect_funcref_targets(program);
    let indirect_call_types = collect_indirect_call_types(program);
    let has_funcref = !funcref_targets.is_empty();

    let mut layout = build_codegen_layout(program)?;
    if has_conc {
        layout.conc_spawn_idx = Some(deduped_ext_count);
        layout.conc_join_idx = Some(deduped_ext_count + 1);
    }
    if stdlib_alloc_module.is_some() {
        let alloc_idx = deduped_ext_count + n_conc_imports;
        layout.allocate_func_idx = Some(alloc_idx);
    }

    // Build funcref table indices
    if has_funcref {
        for (table_idx, func_name) in funcref_targets.iter().enumerate() {
            layout
                .funcref_table_indices
                .insert(*func_name, table_idx as u32);
        }
    }

    let mut module = Module::new();

    // === Type Section ===
    let mut types = TypeSection::new();
    let mut next_type_index: u32 = 0;
    // Track signature→type_index for deduplication
    let mut sig_to_type_idx: HashMap<String, u32> = HashMap::new();

    let mut external_type_indices = Vec::with_capacity(deduped_externals.len());
    for ext in &deduped_externals {
        let params = external_param_types(ext)?;
        let results = external_return_types(ext)?;
        let key = sig_key(&params, &results);
        types.ty().function(params, results);
        sig_to_type_idx.entry(key).or_insert(next_type_index);
        external_type_indices.push(next_type_index);
        next_type_index += 1;
    }

    let mut conc_spawn_type_idx = 0;
    let mut conc_join_type_idx = 0;
    if has_conc {
        types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I32], []);
        conc_spawn_type_idx = next_type_index;
        next_type_index += 1;
        types.ty().function([], []);
        conc_join_type_idx = next_type_index;
        next_type_index += 1;
    }

    // Exception tag type: (i64) -> () — the exception payload is a packed i64
    let mut exn_tag_type_idx = 0;
    if has_eh {
        types.ty().function([ValType::I64], []);
        exn_tag_type_idx = next_type_index;
        next_type_index += 1;
    }

    let mut allocate_type_idx = 0;
    if stdlib_alloc_module.is_some() {
        types.ty().function([ValType::I32], [ValType::I32]);
        allocate_type_idx = next_type_index;
        next_type_index += 1;
    }

    let mut internal_type_indices = Vec::with_capacity(program.functions.len());
    for func in &program.functions {
        let params = func
            .params
            .iter()
            .map(|p| type_to_wasm_valtype(&p.typ))
            .collect::<Result<Vec<_>, _>>()?;
        let results = return_type_to_wasm_result(&func.ret_type)?;
        let key = sig_key(&params, &results);
        types.ty().function(params, results);
        sig_to_type_idx.entry(key).or_insert(next_type_index);
        internal_type_indices.push(next_type_index);
        next_type_index += 1;
    }
    let wasi_cli_run_type_index = next_type_index;
    types.ty().function([], [ValType::I32]);
    next_type_index += 1;

    // Add type signatures for indirect call Arrow types
    for arrow in &indirect_call_types {
        let (params, results) = arrow_type_to_wasm_sig(arrow)?;
        let key = sig_key(&params, &results);
        if !sig_to_type_idx.contains_key(&key) {
            types.ty().function(params, results);
            sig_to_type_idx.insert(key.clone(), next_type_index);
            next_type_index += 1;
        }
        layout
            .indirect_type_indices
            .insert(format!("{:?}", arrow), *sig_to_type_idx.get(&key).unwrap());
    }
    // Also register type indices for any existing signatures that match indirect call patterns
    for arrow in &indirect_call_types {
        let (params, results) = arrow_type_to_wasm_sig(arrow)?;
        let key = sig_key(&params, &results);
        if let Some(&idx) = sig_to_type_idx.get(&key) {
            layout
                .indirect_type_indices
                .insert(format!("{:?}", arrow), idx);
        }
    }

    module.section(&types);

    // === Import Section ===
    let mut imports = ImportSection::new();
    let mut has_imports = false;
    if let MemoryMode::Imported { module: mem_module } = &layout.memory_mode {
        imports.import(
            mem_module,
            MEMORY_EXPORT,
            EntityType::Memory(MemoryType {
                minimum: 1,
                maximum: None,
                memory64: false,
                shared: false,
                page_size_log2: None,
            }),
        );
        has_imports = true;
    }
    for (ext, type_idx) in deduped_externals.iter().zip(external_type_indices.iter()) {
        imports.import(
            ext.wasm_module.as_str(),
            ext.wasm_name.as_str(),
            EntityType::Function(*type_idx),
        );
        has_imports = true;
    }
    if has_conc {
        imports.import(
            CONC_MODULE,
            CONC_SPAWN_NAME,
            EntityType::Function(conc_spawn_type_idx),
        );
        imports.import(
            CONC_MODULE,
            CONC_JOIN_NAME,
            EntityType::Function(conc_join_type_idx),
        );
        has_imports = true;
    }
    if let Some(alloc_module) = &stdlib_alloc_module {
        imports.import(
            alloc_module,
            ALLOCATE_WASM_NAME,
            EntityType::Function(allocate_type_idx),
        );
        has_imports = true;
    }
    if has_imports {
        module.section(&imports);
    }

    // === Function Section ===
    let mut functions = FunctionSection::new();
    for type_idx in internal_type_indices {
        functions.function(type_idx);
    }
    functions.function(wasi_cli_run_type_index);
    module.section(&functions);

    // === Table Section ===
    if has_funcref {
        let table_size = funcref_targets.len() as u64;
        let mut tables = TableSection::new();
        tables.table(TableType {
            element_type: RefType::FUNCREF,
            minimum: table_size,
            maximum: Some(table_size),
            table64: false,
            shared: false,
        });
        module.section(&tables);
    }

    // === Memory Section ===
    if matches!(layout.memory_mode, MemoryMode::Defined) {
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 256,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memories);
    }

    // === Tag Section (WASM EH) ===
    if has_eh {
        let mut tags = TagSection::new();
        tags.tag(TagType {
            kind: TagKind::Exception,
            func_type_idx: exn_tag_type_idx,
        });
        layout.exn_tag_idx = Some(0); // tag index 0
        module.section(&tags);
    }

    // === Global Section ===
    {
        let needs_globals = layout.object_heap_enabled && layout.allocate_func_idx.is_none();
        if needs_globals {
            let mut globals = GlobalSection::new();
            // Heap pointer global (index 0 — OBJECT_HEAP_GLOBAL_INDEX)
            globals.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(layout.heap_base as i32),
            );
            module.section(&globals);
        }
    }

    // === Export Section ===
    let mut exports = ExportSection::new();
    exports.export(ENTRYPOINT, ExportKind::Func, main_idx);
    let wasi_cli_run_func_idx = import_count + program.functions.len() as u32;
    exports.export(WASI_CLI_RUN_EXPORT, ExportKind::Func, wasi_cli_run_func_idx);
    if !matches!(layout.memory_mode, MemoryMode::None) {
        exports.export(MEMORY_EXPORT, ExportKind::Memory, 0);
    }
    for func in &program.functions {
        if func.name.starts_with(CONC_TASK_PREFIX) {
            let idx = internal_function_indices[&func.name];
            exports.export(func.name.as_str(), ExportKind::Func, idx);
        }
    }
    module.section(&exports);

    // === Element Section (funcref table) ===
    if has_funcref {
        let func_indices: Vec<u32> = funcref_targets
            .iter()
            .map(|name| {
                internal_function_indices
                    .get(name)
                    .copied()
                    .unwrap_or_else(|| external_function_indices.get(name).copied().unwrap_or(0))
            })
            .collect();
        let mut elements = ElementSection::new();
        elements.active(
            Some(0),
            &ConstExpr::i32_const(0),
            Elements::Functions(std::borrow::Cow::Borrowed(&func_indices)),
        );
        module.section(&elements);
    }

    // === Code Section ===
    let mut code = CodeSection::new();
    for func in &program.functions {
        let body = compile_function(
            func,
            program,
            &internal_function_indices,
            &external_function_indices,
            &layout,
        )?;
        code.function(&body);
    }
    let run_wrapper = compile_wasi_cli_run_wrapper(main_idx, &main_func.ret_type);
    code.function(&run_wrapper);
    module.section(&code);

    // === Data Section ===
    if !layout.data_segments.is_empty() {
        let mut data = DataSection::new();
        for seg in &layout.data_segments {
            data.active(
                0,
                &ConstExpr::i32_const(seg.offset as i32),
                seg.bytes.clone(),
            );
        }
        module.section(&data);
    }

    // === Name Section (custom) ===
    {
        let mut names = NameSection::new();
        let mut func_names = NameMap::new();
        // Imported functions first (indices 0..import_count)
        let mut idx: u32 = 0;
        for ext in &deduped_externals {
            func_names.append(idx, ext.wasm_name.as_str());
            idx += 1;
        }
        if has_conc {
            func_names.append(idx, CONC_SPAWN_NAME);
            idx += 1;
            func_names.append(idx, CONC_JOIN_NAME);
            idx += 1;
        }
        if stdlib_alloc_module.is_some() {
            func_names.append(idx, ALLOCATE_WASM_NAME);
            idx += 1;
        }
        // Internal functions (indices import_count..import_count+n_funcs)
        for func in &program.functions {
            func_names.append(idx, func.name.as_str());
            idx += 1;
        }
        // WASI CLI run wrapper
        func_names.append(idx, WASI_CLI_RUN_EXPORT);
        names.functions(&func_names);
        module.section(&names);
    }

    Ok(module.finish())
}

fn compile_wasi_cli_run_wrapper(main_idx: u32, main_ret_type: &Type) -> Function {
    let mut body = Function::new(Vec::new());
    body.instruction(&Instruction::Call(main_idx));
    if !matches!(peel_linear(main_ret_type), Type::Unit) {
        body.instruction(&Instruction::Drop);
    }
    body.instruction(&Instruction::I32Const(0));
    body.instruction(&Instruction::End);
    body
}

/// Collect all function names referenced via FuncRef (functions used as values).
fn collect_funcref_targets(program: &LirProgram) -> Vec<Symbol> {
    let mut targets = HashSet::new();
    fn scan_expr(expr: &LirExpr, targets: &mut HashSet<Symbol>) {
        match expr {
            LirExpr::FuncRef { func, .. } => {
                targets.insert(*func);
            }
            LirExpr::Closure { func, captures, .. } => {
                targets.insert(*func);
                for (_, a) in captures {
                    scan_atom(a, targets);
                }
            }
            LirExpr::CallIndirect { callee, args, .. } => {
                scan_atom(callee, targets);
                for (_, a) in args {
                    scan_atom(a, targets);
                }
            }
            LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
                for (_, a) in args {
                    scan_atom(a, targets);
                }
            }
            LirExpr::Binary { lhs, rhs, .. } => {
                scan_atom(lhs, targets);
                scan_atom(rhs, targets);
            }
            LirExpr::Constructor { args, .. } => {
                for a in args {
                    scan_atom(a, targets);
                }
            }
            LirExpr::Record { fields, .. } => {
                for (_, a) in fields {
                    scan_atom(a, targets);
                }
            }
            LirExpr::ObjectTag { value, .. }
            | LirExpr::ObjectField { value, .. }
            | LirExpr::Raise { value, .. } => {
                scan_atom(value, targets);
            }
            LirExpr::ClosureEnvLoad { .. } => {}
            LirExpr::Atom(a) => scan_atom(a, targets),
        }
    }
    fn scan_atom(_atom: &crate::ir::lir::LirAtom, _targets: &mut HashSet<Symbol>) {}
    fn scan_stmt(stmt: &LirStmt, targets: &mut HashSet<Symbol>) {
        match stmt {
            LirStmt::Let { expr, .. } => scan_expr(expr, targets),
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    scan_stmt(s, targets);
                }
                for s in else_body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    scan_stmt(s, targets);
                }
                for s in else_body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                for s in body {
                    scan_stmt(s, targets);
                }
                for s in catch_body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::Conc { .. } => {}
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                for s in cond_stmts {
                    scan_stmt(s, targets);
                }
                for s in body {
                    scan_stmt(s, targets);
                }
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for c in cases {
                    for s in &c.body {
                        scan_stmt(s, targets);
                    }
                }
                for s in default_body {
                    scan_stmt(s, targets);
                }
            }
        }
    }
    for func in &program.functions {
        for stmt in &func.body {
            scan_stmt(stmt, &mut targets);
        }
    }
    let mut result: Vec<Symbol> = targets.into_iter().collect();
    result.sort_by_key(|s| s.to_string());
    result
}

/// Collect all distinct Arrow types used in CallIndirect (for type signature dedup).
fn collect_indirect_call_types(program: &LirProgram) -> Vec<Type> {
    let mut types = Vec::new();
    let mut seen = HashSet::new();
    fn scan_expr(expr: &LirExpr, types: &mut Vec<Type>, seen: &mut HashSet<String>) {
        if let LirExpr::CallIndirect { callee_type, .. } = expr {
            let key = format!("{:?}", callee_type);
            if seen.insert(key) {
                types.push(callee_type.clone());
            }
        }
    }
    fn scan_stmt(stmt: &LirStmt, types: &mut Vec<Type>, seen: &mut HashSet<String>) {
        match stmt {
            LirStmt::Let { expr, .. } => scan_expr(expr, types, seen),
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    scan_stmt(s, types, seen);
                }
                for s in else_body {
                    scan_stmt(s, types, seen);
                }
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                for s in then_body {
                    scan_stmt(s, types, seen);
                }
                for s in else_body {
                    scan_stmt(s, types, seen);
                }
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => {
                for s in body {
                    scan_stmt(s, types, seen);
                }
                for s in catch_body {
                    scan_stmt(s, types, seen);
                }
            }
            LirStmt::Conc { .. } => {}
            LirStmt::Loop {
                cond_stmts, body, ..
            } => {
                for s in cond_stmts {
                    scan_stmt(s, types, seen);
                }
                for s in body {
                    scan_stmt(s, types, seen);
                }
            }
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for c in cases {
                    for s in &c.body {
                        scan_stmt(s, types, seen);
                    }
                }
                for s in default_body {
                    scan_stmt(s, types, seen);
                }
            }
        }
    }
    for func in &program.functions {
        for stmt in &func.body {
            scan_stmt(stmt, &mut types, &mut seen);
        }
    }
    types
}

/// Build a WASM type signature key from an Arrow type for call_indirect.
/// Prepends `i64` (__env) as first param for uniform closure calling convention.
fn arrow_type_to_wasm_sig(arrow: &Type) -> Result<(Vec<ValType>, Vec<ValType>), CodegenError> {
    if let Type::Arrow(params, ret, _, _) = arrow {
        let mut wasm_params: Vec<ValType> = vec![ValType::I64]; // __env
        wasm_params.extend(
            params
                .iter()
                .map(|(_, t)| type_to_wasm_valtype(t))
                .collect::<Result<Vec<_>, _>>()?,
        );
        let wasm_results = return_type_to_wasm_result(ret)?;
        Ok((wasm_params, wasm_results))
    } else {
        // Fallback: treat as (i64, i64) -> i64 (env + one arg)
        Ok((vec![ValType::I64, ValType::I64], vec![ValType::I64]))
    }
}

/// Create a string key for signature deduplication.
fn sig_key(params: &[ValType], results: &[ValType]) -> String {
    format!("{:?}->{:?}", params, results)
}

/// Check if the LIR program needs backtrace instrumentation.
fn program_needs_eh(program: &LirProgram) -> bool {
    fn stmt_needs_bt(stmt: &LirStmt) -> bool {
        match stmt {
            LirStmt::Let { expr, .. } => matches!(expr, LirExpr::Raise { .. }),
            LirStmt::TryCatch { .. } => true,
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => then_body.iter().any(stmt_needs_bt) || else_body.iter().any(stmt_needs_bt),
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => then_body.iter().any(stmt_needs_bt) || else_body.iter().any(stmt_needs_bt),
            LirStmt::Conc { .. } => false,
            LirStmt::Loop {
                cond_stmts, body, ..
            } => cond_stmts.iter().any(stmt_needs_bt) || body.iter().any(stmt_needs_bt),
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                cases.iter().any(|c| c.body.iter().any(stmt_needs_bt))
                    || default_body.iter().any(stmt_needs_bt)
            }
        }
    }
    program
        .functions
        .iter()
        .any(|f| f.body.iter().any(stmt_needs_bt))
}
