mod emit;
mod error;
mod layout;

pub use error::{CodegenError, CompileError, CompileMetrics};

use std::borrow::Cow;
use std::collections::HashMap;

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, CustomSection, DataSection, EntityType, ExportKind,
    ExportSection, Function, FunctionSection, GlobalSection, GlobalType, ImportSection,
    Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};

use crate::constants::{
    Permission, ENTRYPOINT, MEMORY_EXPORT, NEXUS_CAPABILITIES_SECTION, WASI_CLI_RUN_EXPORT,
};
use crate::ir::lir::{
    LirAtom, LirExpr, LirFunction, LirProgram, LirStmt,
};
use crate::lang::ast::{BinaryOp, Program, Type};
use super::passes::hir_build::{build_hir, HirBuildError};
use super::passes::lir_lower::{lower_mir_to_lir, LirLowerError};
use super::passes::mir_lower::{lower_hir_to_mir, MirLowerError};

use emit::{
    compile_external_arg, constructor_tag, emit_alloc_object,
    emit_numeric_coercion, emit_pack_value_to_i64, emit_string_concat, emit_unpack_i64_to_value,
    expr_type, external_param_types, external_return_types, is_string_concat_operator, memarg,
    pack_string, peel_linear, record_tag, return_type_to_wasm_result,
    type_to_wasm_valtype,
};
use layout::{build_codegen_layout, CodegenLayout, MemoryMode};

const STRING_DATA_BASE: u32 = 16;
const OBJECT_HEAP_GLOBAL_INDEX: u32 = 0;
const CONC_MODULE: &str = "nexus:runtime/conc";
const CONC_SPAWN_NAME: &str = "__nx_conc_spawn";
const CONC_JOIN_NAME: &str = "__nx_conc_join";
const CONC_TASK_PREFIX: &str = "__conc_";

#[derive(Debug, Clone, Copy)]
struct LocalInfo {
    index: u32,
    val_type: ValType,
}

#[derive(Debug, Clone, Copy)]
struct FunctionTemps {
    packed_tmp_i64: u32,
    exn_value_i64: u32,
    exn_flag_i32: u32,
    object_ptr_i32: u32,
    concat_lhs_packed_i64: u32,
    concat_rhs_packed_i64: u32,
    concat_lhs_ptr_i32: u32,
    concat_lhs_len_i32: u32,
    concat_rhs_ptr_i32: u32,
    concat_rhs_len_i32: u32,
    concat_out_ptr_i32: u32,
    concat_out_len_i32: u32,
    concat_idx_i32: u32,
}

/// Compiles a parsed Nexus program through HIR -> MIR -> LIR -> WASM pipeline,
/// returning per-pass timing metrics alongside the WASM bytes.
#[tracing::instrument(skip_all, name = "compile_program_to_wasm")]
pub fn compile_program_to_wasm_with_metrics(
    program: &Program,
) -> Result<(Vec<u8>, CompileMetrics), CompileError> {
    use std::time::Instant;

    validate_main_returns_unit(program)?;
    let caps = extract_main_require_ports_from_ast(program);

    let t = Instant::now();
    let hir = build_hir(program).map_err(CompileError::HirBuild)?;
    let hir_build = t.elapsed();

    let t = Instant::now();
    let mir = lower_hir_to_mir(&hir).map_err(CompileError::MirLower)?;
    let mir_lower = t.elapsed();

    let t = Instant::now();
    let lir = lower_mir_to_lir(&mir, &hir.enum_defs).map_err(CompileError::LirLower)?;
    let lir_lower = t.elapsed();

    let t = Instant::now();
    let mut wasm = compile_lir_to_wasm(&lir).map_err(CompileError::Codegen)?;
    let codegen = t.elapsed();

    if !caps.is_empty() {
        append_capabilities_section(&mut wasm, &caps);
    }

    let metrics = CompileMetrics {
        hir_build,
        mir_lower,
        lir_lower,
        codegen,
    };

    Ok((wasm, metrics))
}

/// Compiles a parsed Nexus program through HIR -> MIR -> LIR -> WASM pipeline.
pub fn compile_program_to_wasm(program: &Program) -> Result<Vec<u8>, CompileError> {
    compile_program_to_wasm_with_metrics(program).map(|(wasm, _)| wasm)
}

/// Compiles LIR (in ANF) directly into core WASM bytes.
pub fn compile_lir_to_wasm(program: &LirProgram) -> Result<Vec<u8>, CodegenError> {
    let has_conc = program
        .functions
        .iter()
        .any(|f| f.name.starts_with(CONC_TASK_PREFIX));
    let n_conc_imports: u32 = if has_conc { 2 } else { 0 };
    let import_count = program.externals.len() as u32 + n_conc_imports;

    let mut internal_function_indices = HashMap::new();
    for (idx, func) in program.functions.iter().enumerate() {
        internal_function_indices.insert(func.name.clone(), import_count + idx as u32);
    }
    let main_idx = internal_function_indices
        .get(ENTRYPOINT)
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
            .insert(CONC_SPAWN_NAME.to_string(), program.externals.len() as u32);
        external_function_indices.insert(
            CONC_JOIN_NAME.to_string(),
            program.externals.len() as u32 + 1,
        );
    }

    let mut layout = build_codegen_layout(program)?;
    if has_conc {
        layout.conc_spawn_idx = Some(program.externals.len() as u32);
        layout.conc_join_idx = Some(program.externals.len() as u32 + 1);
    }

    let mut module = Module::new();

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
        // __nx_conc_spawn(func_idx: i32, args_ptr: i32, n_args: i32) -> ()
        types
            .ty()
            .function([ValType::I32, ValType::I32, ValType::I32], []);
        conc_spawn_type_idx = next_type_index;
        next_type_index += 1;
        // __nx_conc_join() -> ()
        types.ty().function([], []);
        conc_join_type_idx = next_type_index;
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
            &ext.wasm_module,
            &ext.wasm_name,
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
    if has_imports {
        module.section(&imports);
    }

    let mut functions = FunctionSection::new();
    for type_idx in internal_type_indices {
        functions.function(type_idx);
    }
    functions.function(wasi_cli_run_type_index);
    module.section(&functions);

    if matches!(layout.memory_mode, MemoryMode::Defined) {
        let mut memories = MemorySection::new();
        // 17 pages (~1.1 MB).  With the single stdlib bundle the app imports
        // memory from the bundle, so this branch is only reached when there
        // are no external imports (MemoryMode::Defined).
        memories.memory(MemoryType {
            minimum: 17,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memories);
    }

    if layout.object_heap_enabled {
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
            exports.export(&func.name, ExportKind::Func, idx);
        }
    }
    module.section(&exports);

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

/// Maps PermFs/PermNet to capability names for the custom section.
fn perm_to_capability(name: &str) -> Option<&'static str> {
    Permission::from_perm_name(name).map(|p| p.cap_name())
}

/// Extracts runtime permission names from main function's require row in the AST.
fn validate_main_returns_unit(program: &Program) -> Result<(), CompileError> {
    use crate::lang::ast::{Expr, TopLevel};
    for def in &program.definitions {
        if let TopLevel::Let(gl) = &def.node {
            if gl.name == ENTRYPOINT {
                if let Expr::Lambda { ret_type, .. } = &gl.value.node {
                    if *ret_type != Type::Unit {
                        return Err(CompileError::MainSignature(format!(
                            "main must return unit, got '{}'",
                            ret_type
                        )));
                    }
                }
                return Ok(());
            }
        }
    }
    Ok(())
}

fn extract_main_require_ports_from_ast(program: &Program) -> Vec<String> {
    use crate::lang::ast::{Expr, TopLevel};
    for def in &program.definitions {
        if let TopLevel::Let(gl) = &def.node {
            if gl.name == ENTRYPOINT {
                if let Expr::Lambda { requires, .. } = &gl.value.node {
                    return match requires {
                        Type::Row(reqs, _) => reqs
                            .iter()
                            .filter_map(|r| match r {
                                Type::UserDefined(name, args) if args.is_empty() => {
                                    perm_to_capability(name).map(|s| s.to_string())
                                }
                                _ => None,
                            })
                            .collect(),
                        _ => vec![],
                    };
                }
            }
        }
    }
    vec![]
}

/// Appends the `nexus:capabilities` custom section to a core WASM module binary.
fn append_capabilities_section(wasm: &mut Vec<u8>, caps: &[String]) {
    let payload = caps.join("\n");
    let section = CustomSection {
        name: Cow::Borrowed(NEXUS_CAPABILITIES_SECTION),
        data: Cow::Borrowed(payload.as_bytes()),
    };
    // Encode the section into a temporary module and extract the raw section bytes.
    let mut tmp = Module::new();
    tmp.section(&section);
    let encoded = tmp.finish();
    // Module preamble is 8 bytes (magic + version). The rest is the section.
    wasm.extend_from_slice(&encoded[8..]);
}

fn compile_wasi_cli_run_wrapper(main_idx: u32, main_ret_type: &Type) -> Function {
    let mut body = Function::new(Vec::new());
    body.instruction(&Instruction::Call(main_idx));
    if !matches!(peel_linear(main_ret_type), Type::Unit) {
        body.instruction(&Instruction::Drop);
    }
    // Canonical ABI for `wasi:cli/run` lowers `result` to i32 where 0 means success.
    body.instruction(&Instruction::I32Const(0));
    body.instruction(&Instruction::End);
    body
}

fn compile_function(
    func: &LirFunction,
    program: &LirProgram,
    internal_indices: &HashMap<String, u32>,
    external_indices: &HashMap<String, u32>,
    layout: &CodegenLayout,
) -> Result<Function, CodegenError> {
    let mut local_map: HashMap<String, LocalInfo> = HashMap::new();
    for (idx, param) in func.params.iter().enumerate() {
        let vt = type_to_wasm_valtype(&param.typ)?;
        local_map.insert(
            param.name.clone(),
            LocalInfo {
                index: idx as u32,
                val_type: vt,
            },
        );
    }

    let mut next_local_index = func.params.len() as u32;
    let mut local_decls_flat = Vec::new();
    collect_stmt_locals(
        &func.body,
        &mut local_map,
        &mut next_local_index,
        &mut local_decls_flat,
    )?;

    let temps = FunctionTemps {
        packed_tmp_i64: next_local_index,
        exn_value_i64: next_local_index + 1,
        exn_flag_i32: next_local_index + 2,
        object_ptr_i32: next_local_index + 3,
        concat_lhs_packed_i64: next_local_index + 4,
        concat_rhs_packed_i64: next_local_index + 5,
        concat_lhs_ptr_i32: next_local_index + 6,
        concat_lhs_len_i32: next_local_index + 7,
        concat_rhs_ptr_i32: next_local_index + 8,
        concat_rhs_len_i32: next_local_index + 9,
        concat_out_ptr_i32: next_local_index + 10,
        concat_out_len_i32: next_local_index + 11,
        concat_idx_i32: next_local_index + 12,
    };
    local_decls_flat.push(ValType::I64);
    local_decls_flat.push(ValType::I64);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I64);
    local_decls_flat.push(ValType::I64);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I32);
    local_decls_flat.push(ValType::I32);

    let mut wasm_locals = Vec::new();
    for vt in local_decls_flat {
        if let Some((last_count, last_ty)) = wasm_locals.last_mut() {
            if *last_ty == vt {
                *last_count += 1;
                continue;
            }
        }
        wasm_locals.push((1, vt));
    }

    let mut out = Function::new(wasm_locals);
    for stmt in &func.body {
        compile_stmt(
            stmt,
            &mut out,
            &local_map,
            program,
            internal_indices,
            external_indices,
            layout,
            &temps,
            &func.ret_type,
            false,
        )?;
    }

    if !matches!(func.ret_type, Type::Unit) {
        compile_atom(&func.ret, &mut out, &local_map, layout)?;
        emit_numeric_coercion(&func.ret.typ(), &func.ret_type, &mut out)?;
    }
    out.instruction(&Instruction::End);

    Ok(out)
}

fn register_local(
    local_map: &mut HashMap<String, LocalInfo>,
    next_local_index: &mut u32,
    local_decls_flat: &mut Vec<ValType>,
    name: &str,
    typ: &Type,
) -> Result<(), CodegenError> {
    if matches!(typ, Type::Unit) {
        return Ok(());
    }
    let vt = type_to_wasm_valtype(typ)?;
    match local_map.get(name) {
        Some(existing) => {
            if existing.val_type != vt {
                return Err(CodegenError::ConflictingLocalTypes {
                    name: name.to_string(),
                });
            }
        }
        None => {
            local_map.insert(
                name.to_string(),
                LocalInfo {
                    index: *next_local_index,
                    val_type: vt,
                },
            );
            *next_local_index += 1;
            local_decls_flat.push(vt);
        }
    }
    Ok(())
}

fn collect_stmt_locals(
    stmts: &[LirStmt],
    local_map: &mut HashMap<String, LocalInfo>,
    next_local_index: &mut u32,
    local_decls_flat: &mut Vec<ValType>,
) -> Result<(), CodegenError> {
    for stmt in stmts {
        match stmt {
            LirStmt::Let { name, typ, .. } => {
                register_local(local_map, next_local_index, local_decls_flat, name, typ)?;
            }
            LirStmt::TryCatch {
                body,
                catch_param,
                catch_param_typ,
                catch_body,
                ..
            } => {
                register_local(
                    local_map,
                    next_local_index,
                    local_decls_flat,
                    catch_param,
                    catch_param_typ,
                )?;
                collect_stmt_locals(body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(catch_body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_locals(then_body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(else_body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_locals(then_body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(else_body, local_map, next_local_index, local_decls_flat)?;
            }
            LirStmt::Conc { .. } => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compile_stmt(
    stmt: &LirStmt,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    program: &LirProgram,
    internal_indices: &HashMap<String, u32>,
    external_indices: &HashMap<String, u32>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
    function_ret_type: &Type,
    in_try: bool,
) -> Result<(), CodegenError> {
    match stmt {
        LirStmt::Let { name, typ, expr } => {
            compile_expr(
                expr,
                out,
                local_map,
                program,
                internal_indices,
                external_indices,
                layout,
                temps,
                in_try,
            )?;
            let et = expr_type(expr);
            if matches!(typ, Type::Unit) {
                if !matches!(et, Type::Unit) {
                    out.instruction(&Instruction::Drop);
                }
                return Ok(());
            }
            emit_numeric_coercion(&et, typ, out)?;
            let local = local_map
                .get(name)
                .ok_or_else(|| CodegenError::ConflictingLocalTypes { name: name.clone() })?;
            out.instruction(&Instruction::LocalSet(local.index));
            Ok(())
        }
        LirStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::If(BlockType::Empty));
            for nested in then_body {
                compile_stmt(
                    nested,
                    out,
                    local_map,
                    program,
                    internal_indices,
                    external_indices,
                    layout,
                    temps,
                    function_ret_type,
                    in_try,
                )?;
            }
            if !else_body.is_empty() {
                out.instruction(&Instruction::Else);
                for nested in else_body {
                    compile_stmt(
                        nested,
                        out,
                        local_map,
                        program,
                        internal_indices,
                        external_indices,
                        layout,
                        temps,
                        function_ret_type,
                        in_try,
                    )?;
                }
            }
            out.instruction(&Instruction::End);
            Ok(())
        }
        LirStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ret_type,
        } => {
            compile_atom(cond, out, local_map, layout)?;
            emit_numeric_coercion(&cond.typ(), &Type::Bool, out)?;
            out.instruction(&Instruction::If(BlockType::Empty));
            for nested in then_body {
                compile_stmt(
                    nested,
                    out,
                    local_map,
                    program,
                    internal_indices,
                    external_indices,
                    layout,
                    temps,
                    function_ret_type,
                    in_try,
                )?;
            }
            compile_atom(then_ret, out, local_map, layout)?;
            emit_numeric_coercion(&then_ret.typ(), ret_type, out)?;
            out.instruction(&Instruction::Return);
            if !else_body.is_empty() || else_ret.is_some() {
                out.instruction(&Instruction::Else);
                for nested in else_body {
                    compile_stmt(
                        nested,
                        out,
                        local_map,
                        program,
                        internal_indices,
                        external_indices,
                        layout,
                        temps,
                        function_ret_type,
                        in_try,
                    )?;
                }
                if let Some(else_ret) = else_ret {
                    compile_atom(else_ret, out, local_map, layout)?;
                    emit_numeric_coercion(&else_ret.typ(), ret_type, out)?;
                    out.instruction(&Instruction::Return);
                }
            }
            out.instruction(&Instruction::End);
            Ok(())
        }
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_param,
            catch_param_typ: _,
            catch_body,
            catch_ret,
        } => {
            let catch_local =
                local_map
                    .get(catch_param)
                    .ok_or_else(|| CodegenError::ConflictingLocalTypes {
                        name: catch_param.clone(),
                    })?;

            out.instruction(&Instruction::I32Const(0));
            out.instruction(&Instruction::LocalSet(temps.exn_flag_i32));

            out.instruction(&Instruction::Block(BlockType::Empty));
            for nested in body {
                compile_stmt(
                    nested,
                    out,
                    local_map,
                    program,
                    internal_indices,
                    external_indices,
                    layout,
                    temps,
                    function_ret_type,
                    true,
                )?;
                out.instruction(&Instruction::LocalGet(temps.exn_flag_i32));
                out.instruction(&Instruction::BrIf(0));
            }
            if let Some(ret) = body_ret {
                compile_atom(ret, out, local_map, layout)?;
                emit_numeric_coercion(&ret.typ(), function_ret_type, out)?;
                out.instruction(&Instruction::Return);
            }
            out.instruction(&Instruction::End);

            out.instruction(&Instruction::LocalGet(temps.exn_flag_i32));
            out.instruction(&Instruction::If(BlockType::Empty));
            out.instruction(&Instruction::LocalGet(temps.exn_value_i64));
            out.instruction(&Instruction::LocalSet(catch_local.index));
            out.instruction(&Instruction::I32Const(0));
            out.instruction(&Instruction::LocalSet(temps.exn_flag_i32));

            if in_try {
                out.instruction(&Instruction::Block(BlockType::Empty));
            }
            for nested in catch_body {
                compile_stmt(
                    nested,
                    out,
                    local_map,
                    program,
                    internal_indices,
                    external_indices,
                    layout,
                    temps,
                    function_ret_type,
                    in_try,
                )?;
                if in_try {
                    out.instruction(&Instruction::LocalGet(temps.exn_flag_i32));
                    out.instruction(&Instruction::BrIf(0));
                }
            }
            if let Some(ret) = catch_ret {
                compile_atom(ret, out, local_map, layout)?;
                emit_numeric_coercion(&ret.typ(), function_ret_type, out)?;
                out.instruction(&Instruction::Return);
            }
            if in_try {
                out.instruction(&Instruction::End);
            }
            out.instruction(&Instruction::End);
            Ok(())
        }
        LirStmt::Conc { tasks } => {
            let spawn_idx = layout
                .conc_spawn_idx
                .expect("conc_spawn_idx must be set for conc blocks");
            let join_idx = layout
                .conc_join_idx
                .expect("conc_join_idx must be set for conc blocks");

            for task in tasks {
                let func_idx = internal_indices
                    .get(&task.func_name)
                    .copied()
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: task.func_name.clone(),
                    })?;
                let n_args = task.args.len() as i32;

                // Allocate space for args on the object heap
                let args_ptr_local = temps.object_ptr_i32;
                out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                out.instruction(&Instruction::LocalSet(args_ptr_local));

                // Write each captured arg as i64 to the heap
                for (i, (_, arg)) in task.args.iter().enumerate() {
                    out.instruction(&Instruction::LocalGet(args_ptr_local));
                    compile_atom(arg, out, local_map, layout)?;
                    // Widen to i64 if needed
                    match arg.typ() {
                        Type::I32 | Type::Bool => {
                            out.instruction(&Instruction::I64ExtendI32U);
                        }
                        Type::F64 => {
                            out.instruction(&Instruction::I64ReinterpretF64);
                        }
                        Type::F32 => {
                            out.instruction(&Instruction::F64PromoteF32);
                            out.instruction(&Instruction::I64ReinterpretF64);
                        }
                        _ => {} // i64, string (packed i64), objects (i64 ptr)
                    }
                    out.instruction(&Instruction::I64Store(MemArg {
                        offset: (i * 8) as u64,
                        align: 3, // 8-byte alignment
                        memory_index: 0,
                    }));
                }

                // Advance heap pointer
                out.instruction(&Instruction::LocalGet(args_ptr_local));
                out.instruction(&Instruction::I32Const(n_args * 8));
                out.instruction(&Instruction::I32Add);
                out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));

                // Call __nx_conc_spawn(func_idx, args_ptr, n_args)
                out.instruction(&Instruction::I32Const(func_idx as i32));
                out.instruction(&Instruction::LocalGet(args_ptr_local));
                out.instruction(&Instruction::I32Const(n_args));
                out.instruction(&Instruction::Call(spawn_idx));
            }

            // Call __nx_conc_join()
            out.instruction(&Instruction::Call(join_idx));
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_expr(
    expr: &LirExpr,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    program: &LirProgram,
    internal_indices: &HashMap<String, u32>,
    external_indices: &HashMap<String, u32>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
    in_try: bool,
) -> Result<(), CodegenError> {
    match expr {
        LirExpr::Atom(atom) => compile_atom(atom, out, local_map, layout),
        LirExpr::Binary { op, lhs, rhs, typ } => {
            if is_string_concat_operator(*op, typ) {
                return emit_string_concat(lhs, rhs, out, local_map, layout, temps);
            }
            let operand_type = binary_operand_type(*op, &lhs.typ(), &rhs.typ())?;
            compile_atom(lhs, out, local_map, layout)?;
            emit_numeric_coercion(&lhs.typ(), &operand_type, out)?;
            compile_atom(rhs, out, local_map, layout)?;
            emit_numeric_coercion(&rhs.typ(), &operand_type, out)?;
            compile_binary(*op, &operand_type, out)
        }
        LirExpr::Call { func, args, .. } => {
            if let Some(callee_idx) = internal_indices.get(func).copied() {
                let callee = program
                    .functions
                    .iter()
                    .find(|f| f.name == *func)
                    .ok_or_else(|| CodegenError::CallTargetNotFound { name: func.clone() })?;

                if args.len() != callee.params.len() {
                    return Err(CodegenError::CallArityMismatch {
                        name: func.clone(),
                        expected: callee.params.len(),
                        got: args.len(),
                    });
                }

                for ((label, atom), param) in args.iter().zip(callee.params.iter()) {
                    if label != &param.label {
                        return Err(CodegenError::CallLabelMismatch {
                            name: func.clone(),
                            expected: param.label.clone(),
                            got: label.clone(),
                        });
                    }
                    compile_atom(atom, out, local_map, layout)?;
                    emit_numeric_coercion(&atom.typ(), &param.typ, out)?;
                }
                out.instruction(&Instruction::Call(callee_idx));
                return Ok(());
            }

            if let Some(callee_idx) = external_indices.get(func).copied() {
                let callee = program
                    .externals
                    .iter()
                    .find(|f| f.name == *func)
                    .ok_or_else(|| CodegenError::CallTargetNotFound { name: func.clone() })?;

                if args.len() != callee.params.len() {
                    return Err(CodegenError::CallArityMismatch {
                        name: func.clone(),
                        expected: callee.params.len(),
                        got: args.len(),
                    });
                }

                for ((label, atom), param) in args.iter().zip(callee.params.iter()) {
                    if label != &param.label {
                        return Err(CodegenError::CallLabelMismatch {
                            name: func.clone(),
                            expected: param.label.clone(),
                            got: label.clone(),
                        });
                    }
                    compile_external_arg(atom, &param.typ, out, local_map, layout, temps)?;
                }

                out.instruction(&Instruction::Call(callee_idx));
                return Ok(());
            }

            Err(CodegenError::CallTargetNotFound { name: func.clone() })
        }
        LirExpr::Constructor { name, args, .. } => {
            emit_alloc_object(out, temps, 1 + args.len(), layout)?;

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Const(constructor_tag(name, args.len())));
            out.instruction(&Instruction::I64Store(memarg(0)));

            for (idx, arg) in args.iter().enumerate() {
                out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                compile_atom(arg, out, local_map, layout)?;
                emit_pack_value_to_i64(&arg.typ(), out)?;
                out.instruction(&Instruction::I64Store(memarg(((idx + 1) * 8) as u64)));
            }

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64ExtendI32U);
            Ok(())
        }
        LirExpr::Record { fields, .. } => {
            let mut field_names: Vec<String> =
                fields.iter().map(|(name, _)| name.clone()).collect();
            field_names.sort();
            let tag = record_tag(&field_names);

            emit_alloc_object(out, temps, 1 + fields.len(), layout)?;

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Const(tag));
            out.instruction(&Instruction::I64Store(memarg(0)));

            for (idx, (_, value)) in fields.iter().enumerate() {
                out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
                compile_atom(value, out, local_map, layout)?;
                emit_pack_value_to_i64(&value.typ(), out)?;
                out.instruction(&Instruction::I64Store(memarg(((idx + 1) * 8) as u64)));
            }

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64ExtendI32U);
            Ok(())
        }
        LirExpr::ObjectTag { value, .. } => {
            compile_atom(value, out, local_map, layout)?;
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Load(memarg(0)));
            Ok(())
        }
        LirExpr::ObjectField { value, index, typ } => {
            compile_atom(value, out, local_map, layout)?;
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Load(memarg(((index + 1) * 8) as u64)));
            emit_unpack_i64_to_value(typ, out)?;
            Ok(())
        }
        LirExpr::Raise { value, .. } => {
            compile_atom(value, out, local_map, layout)?;
            if !matches!(value.typ(), Type::Unit) {
                out.instruction(&Instruction::LocalSet(temps.exn_value_i64));
            } else {
                out.instruction(&Instruction::I64Const(0));
                out.instruction(&Instruction::LocalSet(temps.exn_value_i64));
            }
            out.instruction(&Instruction::I32Const(1));
            out.instruction(&Instruction::LocalSet(temps.exn_flag_i32));
            if !in_try {
                out.instruction(&Instruction::Unreachable);
            }
            Ok(())
        }
    }
}

fn compile_atom(
    atom: &LirAtom,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    layout: &CodegenLayout,
) -> Result<(), CodegenError> {
    match atom {
        LirAtom::Var { name, .. } => {
            let local = local_map
                .get(name)
                .ok_or_else(|| CodegenError::ConflictingLocalTypes { name: name.clone() })?;
            out.instruction(&Instruction::LocalGet(local.index));
            Ok(())
        }
        LirAtom::Int(i) => {
            out.instruction(&Instruction::I64Const(*i));
            Ok(())
        }
        LirAtom::Float(f) => {
            out.instruction(&Instruction::F64Const((*f).into()));
            Ok(())
        }
        LirAtom::Bool(b) => {
            out.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            Ok(())
        }
        LirAtom::String(s) => {
            let packed = layout
                .string_literals
                .get(s)
                .copied()
                .ok_or_else(|| CodegenError::StringLiteralsWithoutMemory)?;
            out.instruction(&Instruction::I64Const(pack_string(packed)));
            Ok(())
        }
        LirAtom::Unit => Ok(()),
    }
}

fn compile_binary(
    op: BinaryOp,
    operand_type: &Type,
    out: &mut Function,
) -> Result<(), CodegenError> {
    match peel_linear(operand_type) {
        Type::I64 => match op {
            BinaryOp::Add => {
                out.instruction(&Instruction::I64Add);
            }
            BinaryOp::Sub => {
                out.instruction(&Instruction::I64Sub);
            }
            BinaryOp::Mul => {
                out.instruction(&Instruction::I64Mul);
            }
            BinaryOp::Div => {
                out.instruction(&Instruction::I64DivS);
            }
            BinaryOp::Mod => {
                out.instruction(&Instruction::I64RemS);
            }
            BinaryOp::BitAnd => {
                out.instruction(&Instruction::I64And);
            }
            BinaryOp::BitOr => {
                out.instruction(&Instruction::I64Or);
            }
            BinaryOp::BitXor => {
                out.instruction(&Instruction::I64Xor);
            }
            BinaryOp::Shl => {
                out.instruction(&Instruction::I64Shl);
            }
            BinaryOp::Shr => {
                out.instruction(&Instruction::I64ShrS);
            }
            BinaryOp::Eq => {
                out.instruction(&Instruction::I64Eq);
            }
            BinaryOp::Ne => {
                out.instruction(&Instruction::I64Ne);
            }
            BinaryOp::Lt => {
                out.instruction(&Instruction::I64LtS);
            }
            BinaryOp::Le => {
                out.instruction(&Instruction::I64LeS);
            }
            BinaryOp::Gt => {
                out.instruction(&Instruction::I64GtS);
            }
            BinaryOp::Ge => {
                out.instruction(&Instruction::I64GeS);
            }
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "i64".to_string(),
                })
            }
        },
        Type::I32 => match op {
            BinaryOp::Add => {
                out.instruction(&Instruction::I32Add);
            }
            BinaryOp::Sub => {
                out.instruction(&Instruction::I32Sub);
            }
            BinaryOp::Mul => {
                out.instruction(&Instruction::I32Mul);
            }
            BinaryOp::Div => {
                out.instruction(&Instruction::I32DivS);
            }
            BinaryOp::Mod => {
                out.instruction(&Instruction::I32RemS);
            }
            BinaryOp::BitAnd => {
                out.instruction(&Instruction::I32And);
            }
            BinaryOp::BitOr => {
                out.instruction(&Instruction::I32Or);
            }
            BinaryOp::BitXor => {
                out.instruction(&Instruction::I32Xor);
            }
            BinaryOp::Shl => {
                out.instruction(&Instruction::I32Shl);
            }
            BinaryOp::Shr => {
                out.instruction(&Instruction::I32ShrS);
            }
            BinaryOp::Eq => {
                out.instruction(&Instruction::I32Eq);
            }
            BinaryOp::Ne => {
                out.instruction(&Instruction::I32Ne);
            }
            BinaryOp::Lt => {
                out.instruction(&Instruction::I32LtS);
            }
            BinaryOp::Le => {
                out.instruction(&Instruction::I32LeS);
            }
            BinaryOp::Gt => {
                out.instruction(&Instruction::I32GtS);
            }
            BinaryOp::Ge => {
                out.instruction(&Instruction::I32GeS);
            }
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "i32".to_string(),
                })
            }
        },
        Type::Bool => match op {
            BinaryOp::Eq => {
                out.instruction(&Instruction::I32Eq);
            }
            BinaryOp::Ne => {
                out.instruction(&Instruction::I32Ne);
            }
            BinaryOp::And => {
                out.instruction(&Instruction::I32And);
            }
            BinaryOp::Or => {
                out.instruction(&Instruction::I32Or);
            }
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "bool".to_string(),
                })
            }
        },
        Type::UserDefined(_, _) | Type::Var(_) | Type::Record(_) => match op {
            BinaryOp::Eq => {
                out.instruction(&Instruction::I64Eq);
            }
            BinaryOp::Ne => {
                out.instruction(&Instruction::I64Ne);
            }
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "user-defined".to_string(),
                })
            }
        },
        Type::F64 => match op {
            BinaryOp::FAdd => {
                out.instruction(&Instruction::F64Add);
            }
            BinaryOp::FSub => {
                out.instruction(&Instruction::F64Sub);
            }
            BinaryOp::FMul => {
                out.instruction(&Instruction::F64Mul);
            }
            BinaryOp::FDiv => {
                out.instruction(&Instruction::F64Div);
            }
            BinaryOp::FEq => {
                out.instruction(&Instruction::F64Eq);
            }
            BinaryOp::FNe => {
                out.instruction(&Instruction::F64Ne);
            }
            BinaryOp::FLt => {
                out.instruction(&Instruction::F64Lt);
            }
            BinaryOp::FLe => {
                out.instruction(&Instruction::F64Le);
            }
            BinaryOp::FGt => {
                out.instruction(&Instruction::F64Gt);
            }
            BinaryOp::FGe => {
                out.instruction(&Instruction::F64Ge);
            }
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "f64".to_string(),
                })
            }
        },
        Type::F32 => match op {
            BinaryOp::FAdd => {
                out.instruction(&Instruction::F32Add);
            }
            BinaryOp::FSub => {
                out.instruction(&Instruction::F32Sub);
            }
            BinaryOp::FMul => {
                out.instruction(&Instruction::F32Mul);
            }
            BinaryOp::FDiv => {
                out.instruction(&Instruction::F32Div);
            }
            BinaryOp::FEq => {
                out.instruction(&Instruction::F32Eq);
            }
            BinaryOp::FNe => {
                out.instruction(&Instruction::F32Ne);
            }
            BinaryOp::FLt => {
                out.instruction(&Instruction::F32Lt);
            }
            BinaryOp::FLe => {
                out.instruction(&Instruction::F32Le);
            }
            BinaryOp::FGt => {
                out.instruction(&Instruction::F32Gt);
            }
            BinaryOp::FGe => {
                out.instruction(&Instruction::F32Ge);
            }
            _ => {
                return Err(CodegenError::UnsupportedBinaryOp {
                    op,
                    operand_type: "f32".to_string(),
                })
            }
        },
        other => {
            return Err(CodegenError::UnsupportedBinaryOp {
                op,
                operand_type: other.to_string(),
            })
        }
    }
    Ok(())
}

fn binary_operand_type(op: BinaryOp, lhs: &Type, rhs: &Type) -> Result<Type, CodegenError> {
    let lhs = peel_linear(lhs);
    let rhs = peel_linear(rhs);
    if matches!(op, BinaryOp::Concat | BinaryOp::Add)
        && matches!(lhs, Type::String)
        && matches!(rhs, Type::String)
    {
        return Ok(Type::String);
    }
    if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
        if matches!(lhs, Type::Bool) && matches!(rhs, Type::Bool) {
            return Ok(Type::Bool);
        }
        if matches!(lhs, Type::UserDefined(_, _) | Type::Var(_))
            && matches!(rhs, Type::UserDefined(_, _) | Type::Var(_))
        {
            return Ok(lhs.clone());
        }
        if matches!(lhs, Type::Record(_)) && matches!(rhs, Type::Record(_)) {
            return Ok(lhs.clone());
        }
    }
    if matches!(op, BinaryOp::And | BinaryOp::Or) {
        if matches!(lhs, Type::Bool) && matches!(rhs, Type::Bool) {
            return Ok(Type::Bool);
        }
    }
    if matches!(
        op,
        BinaryOp::Add
            | BinaryOp::Sub
            | BinaryOp::Mul
            | BinaryOp::Div
            | BinaryOp::Mod
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr
            | BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
    ) {
        if matches!(lhs, Type::I32) || matches!(rhs, Type::I32) {
            return Ok(Type::I32);
        }
        if matches!(lhs, Type::I64) || matches!(rhs, Type::I64) {
            return Ok(Type::I64);
        }
    }
    if op.is_float_op() {
        if matches!(lhs, Type::F32) || matches!(rhs, Type::F32) {
            return Ok(Type::F32);
        }
        if matches!(lhs, Type::F64) || matches!(rhs, Type::F64) {
            return Ok(Type::F64);
        }
    }
    Err(CodegenError::UnsupportedBinaryOpPair {
        op,
        lhs: lhs.to_string(),
        rhs: rhs.to_string(),
    })
}
