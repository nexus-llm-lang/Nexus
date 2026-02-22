use std::collections::{HashMap, HashSet};

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemArg,
    MemorySection, MemoryType, Module, TypeSection, ValType,
};

use crate::lang::ast::{Program, Type};

use super::anf::{AnfAtom, AnfExpr, AnfExternal, AnfFunction, AnfProgram, AnfStmt};
use super::lower::{lower_to_typed_anf, LowerError};

const STRING_DATA_BASE: u32 = 16;
const OBJECT_HEAP_GLOBAL_INDEX: u32 = 0;

#[derive(Debug, Clone, PartialEq)]
pub struct CodegenError {
    pub message: String,
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CodegenError {}

#[derive(Debug)]
pub enum CompileError {
    Lower(LowerError),
    Codegen(CodegenError),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::Lower(e) => write!(f, "{}", e),
            CompileError::Codegen(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for CompileError {}

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

#[derive(Debug, Clone, Copy)]
struct PackedString {
    offset: u32,
    len: u32,
}

#[derive(Debug, Clone)]
struct DataSegment {
    offset: u32,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
enum MemoryMode {
    None,
    Defined,
    Imported { module: String },
}

#[derive(Debug, Clone)]
struct CodegenLayout {
    memory_mode: MemoryMode,
    string_literals: HashMap<String, PackedString>,
    data_segments: Vec<DataSegment>,
    object_heap_enabled: bool,
    heap_base: u32,
}

/// Lowers a parsed Nexus program to typed ANF and compiles it to core wasm bytes.
pub fn compile_program_to_wasm(program: &Program) -> Result<Vec<u8>, CompileError> {
    let anf = lower_to_typed_anf(program).map_err(CompileError::Lower)?;
    compile_typed_anf_to_wasm(&anf).map_err(CompileError::Codegen)
}

/// Compiles typed ANF directly into core wasm bytes.
pub fn compile_typed_anf_to_wasm(program: &AnfProgram) -> Result<Vec<u8>, CodegenError> {
    let mut internal_function_indices = HashMap::new();
    for (idx, func) in program.functions.iter().enumerate() {
        internal_function_indices.insert(
            func.name.clone(),
            program.externals.len() as u32 + idx as u32,
        );
    }
    let main_idx = internal_function_indices
        .get("main")
        .copied()
        .ok_or_else(|| err("main function not found in ANF program"))?;
    let main_func = program
        .functions
        .iter()
        .find(|func| func.name == "main")
        .ok_or_else(|| err("main function body not found in ANF program"))?;

    let mut external_function_indices = HashMap::new();
    for (idx, ext) in program.externals.iter().enumerate() {
        external_function_indices.insert(ext.name.clone(), idx as u32);
    }

    let layout = build_codegen_layout(program)?;

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
            "memory",
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
        memories.memory(MemoryType {
            minimum: 1,
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
    exports.export("main", ExportKind::Func, main_idx);
    let wasi_cli_run_func_idx = program.externals.len() as u32 + program.functions.len() as u32;
    exports.export(
        "wasi:cli/run@0.2.6#run",
        ExportKind::Func,
        wasi_cli_run_func_idx,
    );
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
    func: &AnfFunction,
    program: &AnfProgram,
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
                return Err(err(format!(
                    "variable '{}' has conflicting wasm local types",
                    name
                )));
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
    stmts: &[AnfStmt],
    local_map: &mut HashMap<String, LocalInfo>,
    next_local_index: &mut u32,
    local_decls_flat: &mut Vec<ValType>,
) -> Result<(), CodegenError> {
    for stmt in stmts {
        match stmt {
            AnfStmt::Let { name, typ, .. } => {
                register_local(local_map, next_local_index, local_decls_flat, name, typ)?;
            }
            AnfStmt::TryCatch {
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
            AnfStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_locals(then_body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(else_body, local_map, next_local_index, local_decls_flat)?;
            }
            AnfStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => {
                collect_stmt_locals(then_body, local_map, next_local_index, local_decls_flat)?;
                collect_stmt_locals(else_body, local_map, next_local_index, local_decls_flat)?;
            }
            AnfStmt::Drop(_) => {}
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn compile_stmt(
    stmt: &AnfStmt,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    program: &AnfProgram,
    internal_indices: &HashMap<String, u32>,
    external_indices: &HashMap<String, u32>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
    function_ret_type: &Type,
    in_try: bool,
) -> Result<(), CodegenError> {
    match stmt {
        AnfStmt::Let { name, typ, expr } => {
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
            let expr_type = expr_type(expr);
            if matches!(typ, Type::Unit) {
                if !matches!(expr_type, Type::Unit) {
                    out.instruction(&Instruction::Drop);
                }
                return Ok(());
            }
            emit_numeric_coercion(&expr_type, typ, out)?;
            let local = local_map.get(name).ok_or_else(|| {
                err(format!(
                    "codegen internal error: local '{}' is not allocated",
                    name
                ))
            })?;
            out.instruction(&Instruction::LocalSet(local.index));
            Ok(())
        }
        AnfStmt::Drop(atom) => {
            if !matches!(atom.typ(), Type::Unit) {
                compile_atom(atom, out, local_map, layout)?;
                out.instruction(&Instruction::Drop);
            }
            Ok(())
        }
        AnfStmt::If {
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
        AnfStmt::IfReturn {
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
        AnfStmt::TryCatch {
            body,
            body_ret,
            catch_param,
            catch_param_typ: _,
            catch_body,
            catch_ret,
        } => {
            let catch_local = local_map.get(catch_param).ok_or_else(|| {
                err(format!(
                    "codegen internal error: catch local '{}' is not allocated",
                    catch_param
                ))
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
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_expr(
    expr: &AnfExpr,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    program: &AnfProgram,
    internal_indices: &HashMap<String, u32>,
    external_indices: &HashMap<String, u32>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
    in_try: bool,
) -> Result<(), CodegenError> {
    match expr {
        AnfExpr::Atom(atom) => compile_atom(atom, out, local_map, layout),
        AnfExpr::Binary { op, lhs, rhs, typ } => {
            if is_string_concat_operator(op, typ) {
                return emit_string_concat(lhs, rhs, out, local_map, layout, temps);
            }
            let operand_type = binary_operand_type(op, &lhs.typ(), &rhs.typ())?;
            compile_atom(lhs, out, local_map, layout)?;
            emit_numeric_coercion(&lhs.typ(), &operand_type, out)?;
            compile_atom(rhs, out, local_map, layout)?;
            emit_numeric_coercion(&rhs.typ(), &operand_type, out)?;
            compile_binary(op, &operand_type, out)
        }
        AnfExpr::Call {
            func, args, typ, ..
        } => {
            if let Some(callee_idx) = internal_indices.get(func).copied() {
                let callee = program
                    .functions
                    .iter()
                    .find(|f| f.name == *func)
                    .ok_or_else(|| err(format!("internal call target '{}' not found", func)))?;

                if args.len() != callee.params.len() {
                    return Err(err(format!(
                        "call arity mismatch for '{}': expected {}, got {}",
                        func,
                        callee.params.len(),
                        args.len()
                    )));
                }

                for ((label, atom), param) in args.iter().zip(callee.params.iter()) {
                    if label != &param.label {
                        return Err(err(format!(
                            "call label mismatch for '{}': expected '{}', got '{}'",
                            func, param.label, label
                        )));
                    }
                    compile_atom(atom, out, local_map, layout)?;
                    emit_numeric_coercion(&atom.typ(), &param.typ, out)?;
                }
                out.instruction(&Instruction::Call(callee_idx));
                if matches!(typ, Type::Unit) {
                    return Ok(());
                }
                return Ok(());
            }

            if let Some(callee_idx) = external_indices.get(func).copied() {
                let callee = program
                    .externals
                    .iter()
                    .find(|f| f.name == *func)
                    .ok_or_else(|| err(format!("external call target '{}' not found", func)))?;

                if args.len() != callee.params.len() {
                    return Err(err(format!(
                        "call arity mismatch for '{}': expected {}, got {}",
                        func,
                        callee.params.len(),
                        args.len()
                    )));
                }

                for ((label, atom), param) in args.iter().zip(callee.params.iter()) {
                    if label != &param.label {
                        return Err(err(format!(
                            "call label mismatch for '{}': expected '{}', got '{}'",
                            func, param.label, label
                        )));
                    }
                    compile_external_arg(atom, &param.typ, out, local_map, layout, temps)?;
                }

                out.instruction(&Instruction::Call(callee_idx));
                if matches!(typ, Type::Unit) {
                    return Ok(());
                }
                return Ok(());
            }

            Err(err(format!(
                "call to '{}' is not supported in wasm codegen (callee not found in internal/external lowered symbols)",
                func
            )))
        }
        AnfExpr::Constructor { name, args, .. } => {
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
        AnfExpr::Record { fields, .. } => {
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
        AnfExpr::ObjectTag { value, .. } => {
            compile_atom(value, out, local_map, layout)?;
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Load(memarg(0)));
            Ok(())
        }
        AnfExpr::ObjectField { value, index, typ } => {
            compile_atom(value, out, local_map, layout)?;
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::LocalSet(temps.object_ptr_i32));

            out.instruction(&Instruction::LocalGet(temps.object_ptr_i32));
            out.instruction(&Instruction::I64Load(memarg(((index + 1) * 8) as u64)));
            emit_unpack_i64_to_value(typ, out)?;
            Ok(())
        }
        AnfExpr::Raise { value, .. } => {
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

fn record_tag(sorted_field_names: &[String]) -> i64 {
    let shape = sorted_field_names.join(",");
    hash_tag("rec", &shape, sorted_field_names.len())
}

fn hash_tag(kind: &str, name: &str, arity: usize) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in kind.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for &b in name.as_bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= arity as u64;
    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    hash as i64
}

fn memarg(offset: u64) -> MemArg {
    MemArg {
        offset,
        align: 3,
        memory_index: 0,
    }
}

fn emit_alloc_object(
    out: &mut Function,
    temps: &FunctionTemps,
    words: usize,
    layout: &CodegenLayout,
) -> Result<(), CodegenError> {
    if !layout.object_heap_enabled {
        return Err(err(
            "codegen internal error: object heap allocation requested without object heap",
        ));
    }
    out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
    out.instruction(&Instruction::LocalTee(temps.object_ptr_i32));
    out.instruction(&Instruction::I32Const((words as i32) * 8));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));
    Ok(())
}

fn emit_pack_value_to_i64(typ: &Type, out: &mut Function) -> Result<(), CodegenError> {
    match peel_linear(typ) {
        Type::I64
        | Type::String
        | Type::Array(_)
        | Type::UserDefined(_, _)
        | Type::Var(_)
        | Type::Borrow(_) => Ok(()),
        Type::I32 | Type::Bool => {
            out.instruction(&Instruction::I64ExtendI32S);
            Ok(())
        }
        Type::F64 => {
            out.instruction(&Instruction::I64ReinterpretF64);
            Ok(())
        }
        Type::F32 => {
            out.instruction(&Instruction::I32ReinterpretF32);
            out.instruction(&Instruction::I64ExtendI32U);
            Ok(())
        }
        Type::Unit => {
            out.instruction(&Instruction::Drop);
            out.instruction(&Instruction::I64Const(0));
            Ok(())
        }
        other => Err(err(format!(
            "cannot pack value of type '{}' into object field",
            other
        ))),
    }
}

fn emit_unpack_i64_to_value(typ: &Type, out: &mut Function) -> Result<(), CodegenError> {
    match peel_linear(typ) {
        Type::I64
        | Type::String
        | Type::Array(_)
        | Type::UserDefined(_, _)
        | Type::Var(_)
        | Type::Borrow(_) => Ok(()),
        Type::I32 | Type::Bool => {
            out.instruction(&Instruction::I32WrapI64);
            Ok(())
        }
        Type::F64 => {
            out.instruction(&Instruction::F64ReinterpretI64);
            Ok(())
        }
        Type::F32 => {
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::F32ReinterpretI32);
            Ok(())
        }
        Type::Unit => Err(err("cannot unpack unit from object field")),
        other => Err(err(format!(
            "cannot unpack object field into type '{}'",
            other
        ))),
    }
}

fn compile_external_arg(
    atom: &AnfAtom,
    param_type: &Type,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
) -> Result<(), CodegenError> {
    let param_repr = peel_linear(param_type);
    match param_repr {
        Type::String => {
            if !matches!(peel_linear(&atom.typ()), Type::String) {
                return Err(err(format!(
                    "external call argument type mismatch: expected string, got {}",
                    atom.typ()
                )));
            }
            compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        Type::Array(_) => {
            if !is_array_like_type(&atom.typ()) {
                return Err(err(format!(
                    "external call argument type mismatch: expected array-like value, got {}",
                    atom.typ()
                )));
            }
            compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => {
            if !is_array_like_type(&atom.typ()) {
                return Err(err(format!(
                    "external call argument type mismatch: expected array-like value, got {}",
                    atom.typ()
                )));
            }
            compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        _ => {
            compile_atom(atom, out, local_map, layout)?;
            emit_numeric_coercion(&atom.typ(), param_type, out)?;
            Ok(())
        }
    }
}

fn unpack_packed_i64_to_ptr_len(out: &mut Function, tmp_local: u32) {
    out.instruction(&Instruction::LocalSet(tmp_local));

    out.instruction(&Instruction::LocalGet(tmp_local));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);

    out.instruction(&Instruction::LocalGet(tmp_local));
    out.instruction(&Instruction::I64Const(0xFFFF_FFFFu64 as i64));
    out.instruction(&Instruction::I64And);
    out.instruction(&Instruction::I32WrapI64);
}

fn is_string_concat_operator(op: &str, result_type: &Type) -> bool {
    matches!(op, "++" | "+") && matches!(peel_linear(result_type), Type::String)
}

fn emit_string_concat(
    lhs: &AnfAtom,
    rhs: &AnfAtom,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    layout: &CodegenLayout,
    temps: &FunctionTemps,
) -> Result<(), CodegenError> {
    if !layout.object_heap_enabled {
        return Err(err(
            "codegen internal error: string concat requested without object heap",
        ));
    }
    if !matches!(peel_linear(&lhs.typ()), Type::String)
        || !matches!(peel_linear(&rhs.typ()), Type::String)
    {
        return Err(err(format!(
            "string concat expects string operands, got ({}, {})",
            lhs.typ(),
            rhs.typ()
        )));
    }

    compile_atom(lhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_packed_i64));
    compile_atom(rhs, out, local_map, layout)?;
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_packed_i64));

    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_lhs_len_i32));

    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64ShrU);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_packed_i64));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalSet(temps.concat_rhs_len_i32));

    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalSet(temps.concat_out_len_i32));

    out.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
    out.instruction(&Instruction::LocalTee(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));

    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Block(BlockType::Empty));
    out.instruction(&Instruction::Loop(BlockType::Empty));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::I32GeU);
    out.instruction(&Instruction::BrIf(1));

    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Load8U(memarg_i8()));
    out.instruction(&Instruction::I32Store8(memarg_i8()));

    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Const(1));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Br(0));
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::End);

    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Block(BlockType::Empty));
    out.instruction(&Instruction::Loop(BlockType::Empty));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_len_i32));
    out.instruction(&Instruction::I32GeU);
    out.instruction(&Instruction::BrIf(1));

    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_lhs_len_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalGet(temps.concat_rhs_ptr_i32));
    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Load8U(memarg_i8()));
    out.instruction(&Instruction::I32Store8(memarg_i8()));

    out.instruction(&Instruction::LocalGet(temps.concat_idx_i32));
    out.instruction(&Instruction::I32Const(1));
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::LocalSet(temps.concat_idx_i32));
    out.instruction(&Instruction::Br(0));
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::End);

    out.instruction(&Instruction::LocalGet(temps.concat_out_ptr_i32));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Const(32));
    out.instruction(&Instruction::I64Shl);
    out.instruction(&Instruction::LocalGet(temps.concat_out_len_i32));
    out.instruction(&Instruction::I64ExtendI32U);
    out.instruction(&Instruction::I64Or);
    Ok(())
}

fn expr_type(expr: &AnfExpr) -> Type {
    match expr {
        AnfExpr::Atom(atom) => atom.typ(),
        AnfExpr::Binary { typ, .. } => typ.clone(),
        AnfExpr::Call { typ, .. } => typ.clone(),
        AnfExpr::Constructor { typ, .. } => typ.clone(),
        AnfExpr::Record { typ, .. } => typ.clone(),
        AnfExpr::ObjectTag { typ, .. } => typ.clone(),
        AnfExpr::ObjectField { typ, .. } => typ.clone(),
        AnfExpr::Raise { typ, .. } => typ.clone(),
    }
}

fn compile_atom(
    atom: &AnfAtom,
    out: &mut Function,
    local_map: &HashMap<String, LocalInfo>,
    layout: &CodegenLayout,
) -> Result<(), CodegenError> {
    match atom {
        AnfAtom::Var { name, .. } => {
            let local = local_map
                .get(name)
                .ok_or_else(|| err(format!("unknown local variable '{}'", name)))?;
            out.instruction(&Instruction::LocalGet(local.index));
            Ok(())
        }
        AnfAtom::Int(i) => {
            out.instruction(&Instruction::I64Const(*i));
            Ok(())
        }
        AnfAtom::Float(f) => {
            out.instruction(&Instruction::F64Const((*f).into()));
            Ok(())
        }
        AnfAtom::Bool(b) => {
            out.instruction(&Instruction::I32Const(if *b { 1 } else { 0 }));
            Ok(())
        }
        AnfAtom::String(s) => {
            let packed = layout
                .string_literals
                .get(s)
                .copied()
                .ok_or_else(|| err("codegen internal error: missing string literal layout"))?;
            out.instruction(&Instruction::I64Const(pack_string(packed)));
            Ok(())
        }
        AnfAtom::Unit => Ok(()),
    }
}

fn build_codegen_layout(program: &AnfProgram) -> Result<CodegenLayout, CodegenError> {
    let mut string_literals = Vec::new();
    for func in &program.functions {
        for stmt in &func.body {
            collect_strings_in_stmt(stmt, &mut string_literals);
        }
        collect_strings_in_atom(&func.ret, &mut string_literals);
    }

    let object_heap_enabled = program_uses_object_heap(program);
    let memory_mode =
        choose_memory_mode(program, !string_literals.is_empty(), object_heap_enabled)?;

    let mut literal_map = HashMap::new();
    let mut data_segments = Vec::new();
    let mut seen = HashSet::new();
    let mut next_offset = STRING_DATA_BASE;
    for s in string_literals {
        if !seen.insert(s.clone()) {
            continue;
        }
        let bytes = s.as_bytes().to_vec();
        let len = bytes.len() as u32;
        let packed = PackedString {
            offset: next_offset,
            len,
        };
        literal_map.insert(s, packed);
        data_segments.push(DataSegment {
            offset: next_offset,
            bytes,
        });
        next_offset = next_offset.saturating_add(len);
    }

    if matches!(memory_mode, MemoryMode::None) && !data_segments.is_empty() {
        return Err(err(
            "codegen internal error: string literals exist without memory configuration",
        ));
    }

    let heap_base = align8(next_offset.max(STRING_DATA_BASE));

    Ok(CodegenLayout {
        memory_mode,
        string_literals: literal_map,
        data_segments,
        object_heap_enabled,
        heap_base,
    })
}

fn choose_memory_mode(
    program: &AnfProgram,
    has_string_literals: bool,
    object_heap_enabled: bool,
) -> Result<MemoryMode, CodegenError> {
    let mut modules_with_string_abi = HashSet::new();
    for ext in &program.externals {
        if external_uses_string_abi(ext) {
            modules_with_string_abi.insert(ext.wasm_module.clone());
        }
    }

    if modules_with_string_abi.len() > 1 {
        return if has_string_literals || object_heap_enabled {
            Ok(MemoryMode::Defined)
        } else {
            Ok(MemoryMode::None)
        };
    }

    if let Some(module) = modules_with_string_abi.into_iter().next() {
        return Ok(MemoryMode::Imported { module });
    }

    if has_string_literals || object_heap_enabled {
        Ok(MemoryMode::Defined)
    } else {
        Ok(MemoryMode::None)
    }
}

fn align8(v: u32) -> u32 {
    (v + 7) & !7
}

fn program_uses_object_heap(program: &AnfProgram) -> bool {
    for func in &program.functions {
        for stmt in &func.body {
            if stmt_uses_object_heap(stmt) {
                return true;
            }
        }
    }
    false
}

fn stmt_uses_object_heap(stmt: &AnfStmt) -> bool {
    match stmt {
        AnfStmt::Let { expr, .. } => expr_uses_object_heap(expr),
        AnfStmt::If {
            then_body,
            else_body,
            ..
        } => {
            then_body.iter().any(stmt_uses_object_heap)
                || else_body.iter().any(stmt_uses_object_heap)
        }
        AnfStmt::IfReturn {
            then_body,
            else_body,
            ..
        } => {
            then_body.iter().any(stmt_uses_object_heap)
                || else_body.iter().any(stmt_uses_object_heap)
        }
        AnfStmt::Drop(_) => false,
        AnfStmt::TryCatch {
            body, catch_body, ..
        } => body.iter().any(stmt_uses_object_heap) || catch_body.iter().any(stmt_uses_object_heap),
    }
}

fn expr_uses_object_heap(expr: &AnfExpr) -> bool {
    matches!(
        expr,
        AnfExpr::Constructor { .. }
            | AnfExpr::Record { .. }
            | AnfExpr::ObjectTag { .. }
            | AnfExpr::ObjectField { .. }
    ) || matches!(
        expr,
        AnfExpr::Binary { op, typ, .. } if is_string_concat_operator(op, typ)
    )
}

fn external_uses_string_abi(ext: &AnfExternal) -> bool {
    ext.params
        .iter()
        .any(|p| matches!(peel_linear(&p.typ), Type::String))
        || matches!(peel_linear(&ext.ret_type), Type::String)
}

fn collect_strings_in_stmt(stmt: &AnfStmt, out: &mut Vec<String>) {
    match stmt {
        AnfStmt::Let { expr, .. } => collect_strings_in_expr(expr, out),
        AnfStmt::Drop(atom) => collect_strings_in_atom(atom, out),
        AnfStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            collect_strings_in_atom(cond, out);
            for stmt in then_body {
                collect_strings_in_stmt(stmt, out);
            }
            for stmt in else_body {
                collect_strings_in_stmt(stmt, out);
            }
        }
        AnfStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ..
        } => {
            collect_strings_in_atom(cond, out);
            for stmt in then_body {
                collect_strings_in_stmt(stmt, out);
            }
            collect_strings_in_atom(then_ret, out);
            for stmt in else_body {
                collect_strings_in_stmt(stmt, out);
            }
            if let Some(else_ret) = else_ret {
                collect_strings_in_atom(else_ret, out);
            }
        }
        AnfStmt::TryCatch {
            body,
            body_ret,
            catch_body,
            catch_ret,
            ..
        } => {
            for stmt in body {
                collect_strings_in_stmt(stmt, out);
            }
            if let Some(ret) = body_ret {
                collect_strings_in_atom(ret, out);
            }
            for stmt in catch_body {
                collect_strings_in_stmt(stmt, out);
            }
            if let Some(ret) = catch_ret {
                collect_strings_in_atom(ret, out);
            }
        }
    }
}

fn collect_strings_in_expr(expr: &AnfExpr, out: &mut Vec<String>) {
    match expr {
        AnfExpr::Atom(atom) => collect_strings_in_atom(atom, out),
        AnfExpr::Binary { lhs, rhs, .. } => {
            collect_strings_in_atom(lhs, out);
            collect_strings_in_atom(rhs, out);
        }
        AnfExpr::Call { args, .. } => {
            for (_, atom) in args {
                collect_strings_in_atom(atom, out);
            }
        }
        AnfExpr::Constructor { args, .. } => {
            for atom in args {
                collect_strings_in_atom(atom, out);
            }
        }
        AnfExpr::Record { fields, .. } => {
            for (_, atom) in fields {
                collect_strings_in_atom(atom, out);
            }
        }
        AnfExpr::ObjectTag { value, .. } => collect_strings_in_atom(value, out),
        AnfExpr::ObjectField { value, .. } => collect_strings_in_atom(value, out),
        AnfExpr::Raise { value, .. } => collect_strings_in_atom(value, out),
    }
}

fn collect_strings_in_atom(atom: &AnfAtom, out: &mut Vec<String>) {
    if let AnfAtom::String(s) = atom {
        out.push(s.clone());
    }
}

fn pack_string(s: PackedString) -> i64 {
    (((s.offset as u64) << 32) | (s.len as u64)) as i64
}

fn external_param_types(ext: &AnfExternal) -> Result<Vec<ValType>, CodegenError> {
    let mut out = Vec::new();
    for param in &ext.params {
        match peel_linear(&param.typ) {
            Type::I32 | Type::Bool => out.push(ValType::I32),
            Type::I64 => out.push(ValType::I64),
            Type::F32 => out.push(ValType::F32),
            Type::F64 => out.push(ValType::F64),
            Type::String => {
                out.push(ValType::I32);
                out.push(ValType::I32);
            }
            Type::Array(_) => {
                out.push(ValType::I32);
                out.push(ValType::I32);
            }
            Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => {
                out.push(ValType::I32);
                out.push(ValType::I32);
            }
            other => {
                return Err(err(format!(
                    "external param type '{}' is not supported by current wasm codegen",
                    other
                )))
            }
        }
    }
    Ok(out)
}

fn external_return_types(ext: &AnfExternal) -> Result<Vec<ValType>, CodegenError> {
    match peel_linear(&ext.ret_type) {
        Type::Unit => Ok(vec![]),
        Type::I32 | Type::Bool => Ok(vec![ValType::I32]),
        Type::I64 => Ok(vec![ValType::I64]),
        Type::F32 => Ok(vec![ValType::F32]),
        Type::F64 => Ok(vec![ValType::F64]),
        Type::String => Ok(vec![ValType::I64]),
        other => Err(err(format!(
            "external return type '{}' is not supported by current wasm codegen",
            other
        ))),
    }
}

fn compile_binary(op: &str, operand_type: &Type, out: &mut Function) -> Result<(), CodegenError> {
    match peel_linear(operand_type) {
        Type::I64 => match op {
            "+" => {
                out.instruction(&Instruction::I64Add);
            }
            "-" => {
                out.instruction(&Instruction::I64Sub);
            }
            "*" => {
                out.instruction(&Instruction::I64Mul);
            }
            "/" => {
                out.instruction(&Instruction::I64DivS);
            }
            "==" => {
                out.instruction(&Instruction::I64Eq);
            }
            "!=" => {
                out.instruction(&Instruction::I64Ne);
            }
            "<" => {
                out.instruction(&Instruction::I64LtS);
            }
            "<=" => {
                out.instruction(&Instruction::I64LeS);
            }
            ">" => {
                out.instruction(&Instruction::I64GtS);
            }
            ">=" => {
                out.instruction(&Instruction::I64GeS);
            }
            _ => return Err(err(format!("unsupported i64 binary operator '{}'", op))),
        },
        Type::I32 => match op {
            "+" => {
                out.instruction(&Instruction::I32Add);
            }
            "-" => {
                out.instruction(&Instruction::I32Sub);
            }
            "*" => {
                out.instruction(&Instruction::I32Mul);
            }
            "/" => {
                out.instruction(&Instruction::I32DivS);
            }
            "==" => {
                out.instruction(&Instruction::I32Eq);
            }
            "!=" => {
                out.instruction(&Instruction::I32Ne);
            }
            "<" => {
                out.instruction(&Instruction::I32LtS);
            }
            "<=" => {
                out.instruction(&Instruction::I32LeS);
            }
            ">" => {
                out.instruction(&Instruction::I32GtS);
            }
            ">=" => {
                out.instruction(&Instruction::I32GeS);
            }
            _ => return Err(err(format!("unsupported i32 binary operator '{}'", op))),
        },
        Type::Bool => match op {
            "==" => {
                out.instruction(&Instruction::I32Eq);
            }
            "!=" => {
                out.instruction(&Instruction::I32Ne);
            }
            "&&" => {
                out.instruction(&Instruction::I32And);
            }
            "||" => {
                out.instruction(&Instruction::I32Or);
            }
            _ => return Err(err(format!("unsupported bool binary operator '{}'", op))),
        },
        Type::UserDefined(_, _) | Type::Var(_) | Type::Record(_) => match op {
            "==" => {
                out.instruction(&Instruction::I64Eq);
            }
            "!=" => {
                out.instruction(&Instruction::I64Ne);
            }
            _ => {
                return Err(err(format!(
                    "unsupported user-defined binary operator '{}'",
                    op
                )))
            }
        },
        Type::F64 => match op {
            "+." => {
                out.instruction(&Instruction::F64Add);
            }
            "-." => {
                out.instruction(&Instruction::F64Sub);
            }
            "*." => {
                out.instruction(&Instruction::F64Mul);
            }
            "/." => {
                out.instruction(&Instruction::F64Div);
            }
            "==." => {
                out.instruction(&Instruction::F64Eq);
            }
            "!=." => {
                out.instruction(&Instruction::F64Ne);
            }
            "<." => {
                out.instruction(&Instruction::F64Lt);
            }
            "<=." => {
                out.instruction(&Instruction::F64Le);
            }
            ">." => {
                out.instruction(&Instruction::F64Gt);
            }
            ">=." => {
                out.instruction(&Instruction::F64Ge);
            }
            _ => return Err(err(format!("unsupported f64 binary operator '{}'", op))),
        },
        Type::F32 => match op {
            "+." => {
                out.instruction(&Instruction::F32Add);
            }
            "-." => {
                out.instruction(&Instruction::F32Sub);
            }
            "*." => {
                out.instruction(&Instruction::F32Mul);
            }
            "/." => {
                out.instruction(&Instruction::F32Div);
            }
            "==." => {
                out.instruction(&Instruction::F32Eq);
            }
            "!=." => {
                out.instruction(&Instruction::F32Ne);
            }
            "<." => {
                out.instruction(&Instruction::F32Lt);
            }
            "<=." => {
                out.instruction(&Instruction::F32Le);
            }
            ">." => {
                out.instruction(&Instruction::F32Gt);
            }
            ">=." => {
                out.instruction(&Instruction::F32Ge);
            }
            _ => return Err(err(format!("unsupported f32 binary operator '{}'", op))),
        },
        other => {
            return Err(err(format!(
            "binary operator '{}' with operand type '{}' is not supported by current wasm codegen",
            op, other
        )))
        }
    }
    Ok(())
}

fn binary_operand_type(op: &str, lhs: &Type, rhs: &Type) -> Result<Type, CodegenError> {
    let lhs = peel_linear(lhs);
    let rhs = peel_linear(rhs);
    if matches!(op, "++" | "+") && matches!(lhs, Type::String) && matches!(rhs, Type::String) {
        return Ok(Type::String);
    }
    if matches!(op, "==" | "!=") {
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
    if matches!(op, "&&" | "||") {
        if matches!(lhs, Type::Bool) && matches!(rhs, Type::Bool) {
            return Ok(Type::Bool);
        }
    }
    if matches!(
        op,
        "+" | "-" | "*" | "/" | "==" | "!=" | "<" | "<=" | ">" | ">="
    ) {
        if matches!(lhs, Type::I32) || matches!(rhs, Type::I32) {
            return Ok(Type::I32);
        }
        if matches!(lhs, Type::I64) || matches!(rhs, Type::I64) {
            return Ok(Type::I64);
        }
    }
    if matches!(
        op,
        "+." | "-." | "*." | "/." | "==." | "!=." | "<." | "<=." | ">." | ">=."
    ) {
        if matches!(lhs, Type::F32) || matches!(rhs, Type::F32) {
            return Ok(Type::F32);
        }
        if matches!(lhs, Type::F64) || matches!(rhs, Type::F64) {
            return Ok(Type::F64);
        }
    }
    Err(err(format!(
        "unsupported binary operator '{}' for operand types ({}, {})",
        op, lhs, rhs
    )))
}

fn emit_numeric_coercion(from: &Type, to: &Type, out: &mut Function) -> Result<(), CodegenError> {
    let from = peel_linear(from);
    let to = peel_linear(to);
    if from == to {
        return Ok(());
    }
    if adt_coercion_is_noop(from, to) {
        return Ok(());
    }
    match (from, to) {
        (Type::I64, Type::I32) => {
            out.instruction(&Instruction::I32WrapI64);
            Ok(())
        }
        (Type::I32, Type::I64) => {
            out.instruction(&Instruction::I64ExtendI32S);
            Ok(())
        }
        (Type::F64, Type::F32) => {
            out.instruction(&Instruction::F32DemoteF64);
            Ok(())
        }
        (Type::F32, Type::F64) => {
            out.instruction(&Instruction::F64PromoteF32);
            Ok(())
        }
        _ => Err(err(format!(
            "unsupported numeric coercion from '{}' to '{}'",
            from, to
        ))),
    }
}

fn adt_coercion_is_noop(from: &Type, to: &Type) -> bool {
    match (from, to) {
        (Type::UserDefined(from_name, _), Type::UserDefined(to_name, _)) => from_name == to_name,
        (Type::Record(_), Type::Record(_)) => true,
        (Type::Var(_), Type::Var(_)) => true,
        (Type::Borrow(from_inner), Type::Borrow(to_inner)) => {
            adt_coercion_is_noop(peel_linear(from_inner), peel_linear(to_inner))
        }
        _ => false,
    }
}

fn type_to_wasm_valtype(typ: &Type) -> Result<ValType, CodegenError> {
    match peel_linear(typ) {
        Type::I32 | Type::Bool => Ok(ValType::I32),
        Type::I64 => Ok(ValType::I64),
        Type::F32 => Ok(ValType::F32),
        Type::F64 => Ok(ValType::F64),
        Type::String => Ok(ValType::I64),
        Type::Array(_) => Ok(ValType::I64),
        Type::Record(_) => Ok(ValType::I64),
        Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => Ok(ValType::I64),
        Type::Borrow(inner)
            if matches!(
                peel_linear(inner),
                Type::Record(_) | Type::UserDefined(_, _) | Type::Var(_)
            ) =>
        {
            Ok(ValType::I64)
        }
        Type::UserDefined(_, _) | Type::Var(_) => Ok(ValType::I64),
        Type::Unit => Err(err(
            "unit cannot be represented as a local/param wasm valtype",
        )),
        other => Err(err(format!(
            "type '{}' is not supported by current wasm codegen",
            other
        ))),
    }
}

fn return_type_to_wasm_result(ret: &Type) -> Result<Vec<ValType>, CodegenError> {
    match peel_linear(ret) {
        Type::Unit => Ok(vec![]),
        _ => Ok(vec![type_to_wasm_valtype(ret)?]),
    }
}

fn peel_linear(mut typ: &Type) -> &Type {
    while let Type::Linear(inner) = typ {
        typ = inner;
    }
    typ
}

fn is_array_like_type(typ: &Type) -> bool {
    match peel_linear(typ) {
        Type::Array(_) => true,
        Type::Borrow(inner) => matches!(peel_linear(inner), Type::Array(_)),
        _ => false,
    }
}

#[allow(dead_code)]
fn memarg_i32() -> MemArg {
    MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    }
}

fn memarg_i8() -> MemArg {
    MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }
}

fn err(message: impl Into<String>) -> CodegenError {
    CodegenError {
        message: message.into(),
    }
}
