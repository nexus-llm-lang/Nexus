use std::collections::HashMap;

use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};

use crate::constants::{ENTRYPOINT, MEMORY_EXPORT, WASI_CLI_RUN_EXPORT};
use crate::intern::Symbol;
use crate::ir::lir::{LirExpr, LirProgram, LirStmt};
use crate::types::Type;

use super::emit::{
    external_param_types, external_return_types, peel_linear, return_type_to_wasm_result,
    type_to_wasm_valtype,
};
use super::error::CodegenError;
use super::function::compile_function;
use super::layout::{build_codegen_layout, program_uses_object_heap, MemoryMode};
use super::{
    ALLOCATE_WASM_NAME, BT_FREEZE_NAME, BT_MODULE, BT_POP_NAME, BT_PUSH_NAME, CONC_JOIN_NAME,
    CONC_MODULE, CONC_SPAWN_NAME, CONC_TASK_PREFIX,
};

/// Compiles LIR (in ANF) directly into core WASM bytes.
pub fn compile_lir_to_wasm(program: &LirProgram) -> Result<Vec<u8>, CodegenError> {
    let has_conc = program
        .functions
        .iter()
        .any(|f| f.name.starts_with(CONC_TASK_PREFIX));
    let has_bt = program_needs_backtrace(program);
    let n_conc_imports: u32 = if has_conc { 2 } else { 0 };
    let n_bt_imports: u32 = if has_bt { 3 } else { 0 };

    let stdlib_alloc_module = if program_uses_object_heap(program) {
        program
            .externals
            .iter()
            .find(|ext| ext.wasm_module.ends_with("stdlib.wasm"))
            .map(|ext| ext.wasm_module.to_string())
    } else {
        None
    };
    let n_alloc_imports: u32 = if stdlib_alloc_module.is_some() {
        1
    } else {
        0
    };

    let import_count =
        program.externals.len() as u32 + n_conc_imports + n_bt_imports + n_alloc_imports;

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

    let mut external_function_indices = HashMap::new();
    for (idx, ext) in program.externals.iter().enumerate() {
        external_function_indices.insert(ext.name.clone(), idx as u32);
    }
    if has_conc {
        external_function_indices
            .insert(Symbol::from(CONC_SPAWN_NAME), program.externals.len() as u32);
        external_function_indices.insert(
            Symbol::from(CONC_JOIN_NAME),
            program.externals.len() as u32 + 1,
        );
    }

    let mut layout = build_codegen_layout(program)?;
    if has_conc {
        layout.conc_spawn_idx = Some(program.externals.len() as u32);
        layout.conc_join_idx = Some(program.externals.len() as u32 + 1);
    }
    if has_bt {
        let base = program.externals.len() as u32 + n_conc_imports;
        layout.bt_push_idx = Some(base);
        layout.bt_pop_idx = Some(base + 1);
        layout.bt_freeze_idx = Some(base + 2);
    }
    if stdlib_alloc_module.is_some() {
        let alloc_idx = program.externals.len() as u32 + n_conc_imports + n_bt_imports;
        layout.allocate_func_idx = Some(alloc_idx);
    }

    let mut module = Module::new();

    // === Type Section ===
    let mut types = TypeSection::new();
    let mut next_type_index: u32 = 0;

    let mut external_type_indices = Vec::with_capacity(program.externals.len());
    for ext in &program.externals {
        let params = external_param_types(ext)?;
        let results = external_return_types(ext)?;
        types.ty().function(params, results);
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

    let mut bt_push_type_idx = 0;
    let mut bt_void_type_idx = 0;
    if has_bt {
        types.ty().function([ValType::I64], []);
        bt_push_type_idx = next_type_index;
        next_type_index += 1;
        types.ty().function([], []);
        bt_void_type_idx = next_type_index;
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
        types.ty().function(params, results);
        internal_type_indices.push(next_type_index);
        next_type_index += 1;
    }
    let wasi_cli_run_type_index = next_type_index;
    types.ty().function([], [ValType::I32]);
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
    for (ext, type_idx) in program.externals.iter().zip(external_type_indices.iter()) {
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
    if has_bt {
        imports.import(
            BT_MODULE,
            BT_PUSH_NAME,
            EntityType::Function(bt_push_type_idx),
        );
        imports.import(
            BT_MODULE,
            BT_POP_NAME,
            EntityType::Function(bt_void_type_idx),
        );
        imports.import(
            BT_MODULE,
            BT_FREEZE_NAME,
            EntityType::Function(bt_void_type_idx),
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

    // === Global Section ===
    if layout.object_heap_enabled && layout.allocate_func_idx.is_none() {
        let mut globals = GlobalSection::new();
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

/// Check if the LIR program needs backtrace instrumentation.
fn program_needs_backtrace(program: &LirProgram) -> bool {
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
        }
    }
    program
        .functions
        .iter()
        .any(|f| f.body.iter().any(stmt_needs_bt))
}
