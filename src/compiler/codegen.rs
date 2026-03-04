use std::collections::{HashMap, HashSet};

use std::borrow::Cow;

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, CustomSection, DataSection, EntityType, ExportKind,
    ExportSection, Function, FunctionSection, GlobalSection, GlobalType, ImportSection,
    Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};

use crate::constants::{Permission, ENTRYPOINT, MEMORY_EXPORT, NEXUS_CAPABILITIES_SECTION, WASI_CLI_RUN_EXPORT};
use crate::ir::lir::{LirAtom, LirExpr, LirFunction, LirProgram, LirStmt};
use crate::ir::mir::EvidenceTable;
use crate::lang::ast::{BinaryOp, Program, Type};

use super::anf::{AnfAtom, AnfExpr, AnfExternal, AnfFunction, AnfProgram, AnfStmt};
use super::passes::hir_build::{build_hir, HirBuildError};
use super::passes::lir_lower::{lower_mir_to_lir, LirLowerError};
use super::passes::mir_lower::{lower_hir_to_mir, MirLowerError};

const STRING_DATA_BASE: u32 = 16;
const OBJECT_HEAP_GLOBAL_INDEX: u32 = 0;

#[derive(Debug, Clone, PartialEq)]
pub enum CodegenError {
    /// E2001: main function not found in ANF program
    MissingMain,
    /// E2002: unsupported binary operator for operand type
    UnsupportedBinaryOp { op: BinaryOp, operand_type: String },
    /// E2003: unsupported binary operator for operand type pair
    UnsupportedBinaryOpPair { op: BinaryOp, lhs: String, rhs: String },
    /// E2004: unsupported wasm type
    UnsupportedWasmType { typ: String },
    /// E2005: unit cannot be represented as wasm valtype
    UnitWasmType,
    /// E2006: unsupported numeric coercion
    UnsupportedCoercion { from_type: String, to_type: String },
    /// E2007: call target not found
    CallTargetNotFound { name: String },
    /// E2008: call arity mismatch
    CallArityMismatch { name: String, expected: usize, got: usize },
    /// E2009: call label mismatch
    CallLabelMismatch { name: String, expected: String, got: String },
    /// E2010: conflicting wasm local types
    ConflictingLocalTypes { name: String },
    /// E2011: object heap not enabled
    ObjectHeapRequired { context: &'static str },
    /// E2012: cannot pack value type into object field
    UnsupportedPack { typ: String },
    /// E2013: cannot unpack object field into type
    UnsupportedUnpack { typ: String },
    /// E2014: external param type not supported
    UnsupportedExternalParamType { typ: String },
    /// E2015: external return type not supported
    UnsupportedExternalReturnType { typ: String },
    /// E2016: external call argument type mismatch
    ExternalArgTypeMismatch { expected: String, got: String },
    /// E2017: string concat expects string operands
    StringConcatTypeMismatch { lhs: String, rhs: String },
    /// E2018: string literals exist without memory configuration
    StringLiteralsWithoutMemory,
}

impl CodegenError {
    pub fn code(&self) -> &'static str {
        match self {
            CodegenError::MissingMain => "E2001",
            CodegenError::UnsupportedBinaryOp { .. } => "E2002",
            CodegenError::UnsupportedBinaryOpPair { .. } => "E2003",
            CodegenError::UnsupportedWasmType { .. } => "E2004",
            CodegenError::UnitWasmType => "E2005",
            CodegenError::UnsupportedCoercion { .. } => "E2006",
            CodegenError::CallTargetNotFound { .. } => "E2007",
            CodegenError::CallArityMismatch { .. } => "E2008",
            CodegenError::CallLabelMismatch { .. } => "E2009",
            CodegenError::ConflictingLocalTypes { .. } => "E2010",
            CodegenError::ObjectHeapRequired { .. } => "E2011",
            CodegenError::UnsupportedPack { .. } => "E2012",
            CodegenError::UnsupportedUnpack { .. } => "E2013",
            CodegenError::UnsupportedExternalParamType { .. } => "E2014",
            CodegenError::UnsupportedExternalReturnType { .. } => "E2015",
            CodegenError::ExternalArgTypeMismatch { .. } => "E2016",
            CodegenError::StringConcatTypeMismatch { .. } => "E2017",
            CodegenError::StringLiteralsWithoutMemory => "E2018",
        }
    }
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = self.code();
        let msg = match self {
            CodegenError::MissingMain => {
                "main function not found in ANF program".to_string()
            }
            CodegenError::UnsupportedBinaryOp { op, operand_type } => {
                format!("unsupported {} binary operator '{}'", operand_type, op)
            }
            CodegenError::UnsupportedBinaryOpPair { op, lhs, rhs } => {
                format!("unsupported binary operator '{}' for operand types ({}, {})", op, lhs, rhs)
            }
            CodegenError::UnsupportedWasmType { typ } => {
                format!("type '{}' is not supported by current wasm codegen", typ)
            }
            CodegenError::UnitWasmType => {
                "unit cannot be represented as a local/param wasm valtype".to_string()
            }
            CodegenError::UnsupportedCoercion { from_type, to_type } => {
                format!("unsupported numeric coercion from '{}' to '{}'", from_type, to_type)
            }
            CodegenError::CallTargetNotFound { name } => {
                format!("call target '{}' not found in lowered symbols", name)
            }
            CodegenError::CallArityMismatch { name, expected, got } => {
                format!("call arity mismatch for '{}': expected {}, got {}", name, expected, got)
            }
            CodegenError::CallLabelMismatch { name, expected, got } => {
                format!("call label mismatch for '{}': expected '{}', got '{}'", name, expected, got)
            }
            CodegenError::ConflictingLocalTypes { name } => {
                format!("variable '{}' has conflicting wasm local types", name)
            }
            CodegenError::ObjectHeapRequired { context } => {
                format!("{} requested without object heap", context)
            }
            CodegenError::UnsupportedPack { typ } => {
                format!("cannot pack value of type '{}' into object field", typ)
            }
            CodegenError::UnsupportedUnpack { typ } => {
                format!("cannot unpack object field into type '{}'", typ)
            }
            CodegenError::UnsupportedExternalParamType { typ } => {
                format!("external param type '{}' is not supported by current wasm codegen", typ)
            }
            CodegenError::UnsupportedExternalReturnType { typ } => {
                format!("external return type '{}' is not supported by current wasm codegen", typ)
            }
            CodegenError::ExternalArgTypeMismatch { expected, got } => {
                format!("external call argument type mismatch: expected {}, got {}", expected, got)
            }
            CodegenError::StringConcatTypeMismatch { lhs, rhs } => {
                format!("string concat expects string operands, got ({}, {})", lhs, rhs)
            }
            CodegenError::StringLiteralsWithoutMemory => {
                "string literals exist without memory configuration".to_string()
            }
        };
        write!(f, "internal compiler error: {} [{}] (this is a bug; please report it)", msg, code)
    }
}

impl std::error::Error for CodegenError {}

#[derive(Debug)]
pub enum CompileError {
    HirBuild(HirBuildError),
    MirLower(MirLowerError),
    LirLower(LirLowerError),
    Codegen(CodegenError),
    MainSignature(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::HirBuild(e) => write!(f, "{}", e),
            CompileError::MirLower(e) => write!(f, "{}", e),
            CompileError::LirLower(e) => write!(f, "{}", e),
            CompileError::Codegen(e) => write!(f, "{}", e),
            CompileError::MainSignature(msg) => write!(f, "{}", msg),
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

/// Compiles a parsed Nexus program through HIR → MIR → LIR → WASM pipeline.
#[tracing::instrument(skip_all, name = "compile_program_to_wasm")]
pub fn compile_program_to_wasm(program: &Program) -> Result<Vec<u8>, CompileError> {
    validate_main_returns_unit(program)?;

    // Extract main's require ports from the AST before lowering
    // (the lowering pipeline currently drops the requires clause).
    let caps = extract_main_require_ports_from_ast(program);

    let hir = build_hir(program).map_err(CompileError::HirBuild)?;
    let mir = lower_hir_to_mir(&hir).map_err(CompileError::MirLower)?;
    let lir = lower_mir_to_lir(&mir).map_err(CompileError::LirLower)?;
    let mut wasm = compile_lir_to_wasm(&lir, &mir.evidence_table).map_err(CompileError::Codegen)?;

    // Append nexus:capabilities section from AST requires, since the
    // lowering pipeline doesn't preserve requires through to ANF.
    if !caps.is_empty() {
        append_capabilities_section(&mut wasm, &caps);
    }

    Ok(wasm)
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
    let wasi_cli_run_func_idx = program.externals.len() as u32 + program.functions.len() as u32;
    exports.export(
        WASI_CLI_RUN_EXPORT,
        ExportKind::Func,
        wasi_cli_run_func_idx,
    );
    if matches!(layout.memory_mode, MemoryMode::Defined) {
        exports.export(MEMORY_EXPORT, ExportKind::Memory, 0);
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
                CodegenError::ConflictingLocalTypes {
                    name: name.clone(),
                }
            })?;
            out.instruction(&Instruction::LocalSet(local.index));
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
                CodegenError::ConflictingLocalTypes {
                    name: catch_param.clone(),
                }
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
        AnfExpr::Call {
            func, args, typ, ..
        } => {
            if let Some(callee_idx) = internal_indices.get(func).copied() {
                let callee = program
                    .functions
                    .iter()
                    .find(|f| f.name == *func)
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: func.clone(),
                    })?;

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
                    .ok_or_else(|| CodegenError::CallTargetNotFound {
                        name: func.clone(),
                    })?;

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
                if matches!(typ, Type::Unit) {
                    return Ok(());
                }
                return Ok(());
            }

            Err(CodegenError::CallTargetNotFound {
                name: func.clone(),
            })
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
        return Err(CodegenError::ObjectHeapRequired {
            context: "object heap allocation",
        });
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
        other => Err(CodegenError::UnsupportedPack {
            typ: other.to_string(),
        }),
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
        Type::Unit => Err(CodegenError::UnsupportedUnpack {
            typ: "unit".to_string(),
        }),
        other => Err(CodegenError::UnsupportedUnpack {
            typ: other.to_string(),
        }),
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
                return Err(CodegenError::ExternalArgTypeMismatch {
                    expected: "string".to_string(),
                    got: atom.typ().to_string(),
                });
            }
            compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        Type::Array(_) => {
            if !is_array_like_type(&atom.typ()) {
                return Err(CodegenError::ExternalArgTypeMismatch {
                    expected: "array-like value".to_string(),
                    got: atom.typ().to_string(),
                });
            }
            compile_atom(atom, out, local_map, layout)?;
            unpack_packed_i64_to_ptr_len(out, temps.packed_tmp_i64);
            Ok(())
        }
        Type::Borrow(inner) if matches!(peel_linear(inner), Type::Array(_)) => {
            if !is_array_like_type(&atom.typ()) {
                return Err(CodegenError::ExternalArgTypeMismatch {
                    expected: "array-like value".to_string(),
                    got: atom.typ().to_string(),
                });
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

fn is_string_concat_operator(op: BinaryOp, result_type: &Type) -> bool {
    matches!(op, BinaryOp::Concat | BinaryOp::Add) && matches!(peel_linear(result_type), Type::String)
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
        return Err(CodegenError::ObjectHeapRequired {
            context: "string concat",
        });
    }
    if !matches!(peel_linear(&lhs.typ()), Type::String)
        || !matches!(peel_linear(&rhs.typ()), Type::String)
    {
        return Err(CodegenError::StringConcatTypeMismatch {
            lhs: lhs.typ().to_string(),
            rhs: rhs.typ().to_string(),
        });
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
                .ok_or_else(|| CodegenError::ConflictingLocalTypes {
                    name: name.clone(),
                })?;
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
                .ok_or_else(|| CodegenError::StringLiteralsWithoutMemory)?;
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
        return Err(CodegenError::StringLiteralsWithoutMemory);
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
        AnfExpr::Binary { op, typ, .. } if is_string_concat_operator(*op, typ)
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
                return Err(CodegenError::UnsupportedExternalParamType {
                    typ: other.to_string(),
                })
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
        other => Err(CodegenError::UnsupportedExternalReturnType {
            typ: other.to_string(),
        }),
    }
}

fn compile_binary(op: BinaryOp, operand_type: &Type, out: &mut Function) -> Result<(), CodegenError> {
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
            _ => return Err(CodegenError::UnsupportedBinaryOp { op, operand_type: "i64".to_string() }),
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
            _ => return Err(CodegenError::UnsupportedBinaryOp { op, operand_type: "i32".to_string() }),
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
            _ => return Err(CodegenError::UnsupportedBinaryOp { op, operand_type: "bool".to_string() }),
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
            _ => return Err(CodegenError::UnsupportedBinaryOp { op, operand_type: "f64".to_string() }),
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
            _ => return Err(CodegenError::UnsupportedBinaryOp { op, operand_type: "f32".to_string() }),
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

fn emit_numeric_coercion(from: &Type, to: &Type, out: &mut Function) -> Result<(), CodegenError> {
    let from = peel_linear(from);
    let to = peel_linear(to);
    if from == to {
        return Ok(());
    }
    if adt_coercion_is_noop(from, to) {
        return Ok(());
    }
    // Unit → anything is a no-op in certain control-flow contexts
    // (e.g. void-returning call used where a value is expected)
    if matches!(from, Type::Unit) || matches!(to, Type::Unit) {
        return Ok(());
    }
    // If both types map to the same wasm valtype, no instruction needed
    // (e.g. i64 ↔ UserDefined("Handle", []) — both are ValType::I64)
    if let (Ok(wf), Ok(wt)) = (type_to_wasm_valtype(from), type_to_wasm_valtype(to)) {
        if wf == wt {
            return Ok(());
        }
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
        _ => Err(CodegenError::UnsupportedCoercion {
            from_type: from.to_string(),
            to_type: to.to_string(),
        }),
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
        Type::Unit => Err(CodegenError::UnitWasmType),
        other => Err(CodegenError::UnsupportedWasmType {
            typ: other.to_string(),
        }),
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

// ============================================================
// LIR Codegen: compile LirProgram with evidence table support
// ============================================================

/// Compile a LIR program to WASM bytes with evidence-passing support.
pub fn compile_lir_to_wasm(
    program: &LirProgram,
    evidence_table: &EvidenceTable,
) -> Result<Vec<u8>, CodegenError> {
    // Convert LIR to ANF for reuse of existing codegen
    let anf = lir_program_to_anf(program);
    let wasm = compile_typed_anf_to_wasm(&anf)?;

    // If evidence table is empty, no funcref table needed
    if evidence_table.entries.is_empty() {
        return Ok(wasm);
    }

    // TODO: In Phase 9 wiring, inject funcref table into the WASM module.
    // For now, return the base WASM. The funcref table will be added when
    // the full pipeline is wired.
    Ok(wasm)
}

/// Convert LIR program to ANF program for codegen reuse.
/// Evidence params become regular params, CallIndirect becomes Call to a
/// resolved function name (static dispatch for now).
fn lir_program_to_anf(program: &LirProgram) -> AnfProgram {
    let functions = program
        .functions
        .iter()
        .map(|f| lir_function_to_anf(f))
        .collect();
    let externals = program
        .externals
        .iter()
        .map(|e| AnfExternal {
            name: e.name.clone(),
            wasm_module: e.wasm_module.clone(),
            wasm_name: e.wasm_name.clone(),
            params: e
                .params
                .iter()
                .map(|p| super::anf::AnfParam {
                    label: p.label.clone(),
                    name: p.name.clone(),
                    typ: p.typ.clone(),
                })
                .collect(),
            ret_type: e.ret_type.clone(),
            effects: e.effects.clone(),
        })
        .collect();

    AnfProgram {
        functions,
        externals,
    }
}

fn lir_function_to_anf(func: &LirFunction) -> AnfFunction {
    let mut params: Vec<super::anf::AnfParam> = func
        .params
        .iter()
        .map(|p| super::anf::AnfParam {
            label: p.label.clone(),
            name: p.name.clone(),
            typ: p.typ.clone(),
        })
        .collect();

    // Add evidence params as additional i32 params
    for ep in &func.evidence_params {
        params.push(super::anf::AnfParam {
            label: ep.label.clone(),
            name: ep.name.clone(),
            typ: Type::I32,
        });
    }

    let body = func.body.iter().map(|s| lir_stmt_to_anf(s)).collect();
    let ret = lir_atom_to_anf(&func.ret);

    AnfFunction {
        name: func.name.clone(),
        params,
        ret_type: func.ret_type.clone(),
        requires: func.requires.clone(),
        effects: func.effects.clone(),
        body,
        ret,
    }
}

fn lir_stmt_to_anf(stmt: &LirStmt) -> AnfStmt {
    match stmt {
        LirStmt::Let { name, typ, expr } => AnfStmt::Let {
            name: name.clone(),
            typ: typ.clone(),
            expr: lir_expr_to_anf(expr),
        },
        LirStmt::If {
            cond,
            then_body,
            else_body,
        } => AnfStmt::If {
            cond: lir_atom_to_anf(cond),
            then_body: then_body.iter().map(|s| lir_stmt_to_anf(s)).collect(),
            else_body: else_body.iter().map(|s| lir_stmt_to_anf(s)).collect(),
        },
        LirStmt::IfReturn {
            cond,
            then_body,
            then_ret,
            else_body,
            else_ret,
            ret_type,
        } => AnfStmt::IfReturn {
            cond: lir_atom_to_anf(cond),
            then_body: then_body.iter().map(|s| lir_stmt_to_anf(s)).collect(),
            then_ret: lir_atom_to_anf(then_ret),
            else_body: else_body.iter().map(|s| lir_stmt_to_anf(s)).collect(),
            else_ret: else_ret.as_ref().map(|a| lir_atom_to_anf(a)),
            ret_type: ret_type.clone(),
        },
        LirStmt::TryCatch {
            body,
            body_ret,
            catch_param,
            catch_param_typ,
            catch_body,
            catch_ret,
        } => AnfStmt::TryCatch {
            body: body.iter().map(|s| lir_stmt_to_anf(s)).collect(),
            body_ret: body_ret.as_ref().map(|a| lir_atom_to_anf(a)),
            catch_param: catch_param.clone(),
            catch_param_typ: catch_param_typ.clone(),
            catch_body: catch_body.iter().map(|s| lir_stmt_to_anf(s)).collect(),
            catch_ret: catch_ret.as_ref().map(|a| lir_atom_to_anf(a)),
        },
    }
}

fn lir_expr_to_anf(expr: &LirExpr) -> AnfExpr {
    match expr {
        LirExpr::Atom(atom) => AnfExpr::Atom(lir_atom_to_anf(atom)),
        LirExpr::Binary { op, lhs, rhs, typ } => AnfExpr::Binary {
            op: *op,
            lhs: lir_atom_to_anf(lhs),
            rhs: lir_atom_to_anf(rhs),
            typ: typ.clone(),
        },
        LirExpr::Call { func, args, typ } => AnfExpr::Call {
            func: func.clone(),
            args: args
                .iter()
                .map(|(l, a)| (l.clone(), lir_atom_to_anf(a)))
                .collect(),
            typ: typ.clone(),
        },
        LirExpr::Constructor { name, args, typ } => AnfExpr::Constructor {
            name: name.clone(),
            args: args.iter().map(|a| lir_atom_to_anf(a)).collect(),
            typ: typ.clone(),
        },
        LirExpr::Record { fields, typ } => AnfExpr::Record {
            fields: fields
                .iter()
                .map(|(n, a)| (n.clone(), lir_atom_to_anf(a)))
                .collect(),
            typ: typ.clone(),
        },
        LirExpr::ObjectTag { value, typ } => AnfExpr::ObjectTag {
            value: lir_atom_to_anf(value),
            typ: typ.clone(),
        },
        LirExpr::ObjectField { value, index, typ } => AnfExpr::ObjectField {
            value: lir_atom_to_anf(value),
            index: *index,
            typ: typ.clone(),
        },
        LirExpr::Raise { value, typ } => AnfExpr::Raise {
            value: lir_atom_to_anf(value),
            typ: typ.clone(),
        },
    }
}

fn lir_atom_to_anf(atom: &LirAtom) -> AnfAtom {
    match atom {
        LirAtom::Var { name, typ } => AnfAtom::Var {
            name: name.clone(),
            typ: typ.clone(),
        },
        LirAtom::Int(i) => AnfAtom::Int(*i),
        LirAtom::Float(f) => AnfAtom::Float(*f),
        LirAtom::Bool(b) => AnfAtom::Bool(*b),
        LirAtom::String(s) => AnfAtom::String(s.clone()),
        LirAtom::Unit => AnfAtom::Unit,
    }
}

