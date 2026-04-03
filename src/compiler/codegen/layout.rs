use std::collections::{HashMap, HashSet};

use crate::intern::Symbol;
use crate::ir::lir::{LirAtom, LirExpr, LirExternal, LirProgram, LirStmt};
use crate::types::Type;

use super::emit::peel_linear;
use super::error::CodegenError;
use super::string::is_string_concat_operator;
use super::STRING_DATA_BASE;

#[derive(Debug, Clone, Copy)]
pub(super) struct PackedString {
    pub(super) offset: u32,
    pub(super) len: u32,
}

#[derive(Debug, Clone)]
pub(super) struct DataSegment {
    pub(super) offset: u32,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(super) enum MemoryMode {
    None,
    Defined,
    Imported { module: String },
}

#[derive(Debug, Clone)]
pub(super) struct CodegenLayout {
    pub(super) memory_mode: MemoryMode,
    pub(super) string_literals: HashMap<String, PackedString>,
    pub(super) data_segments: Vec<DataSegment>,
    pub(super) object_heap_enabled: bool,
    pub(super) heap_base: u32,
    pub(super) allocate_func_idx: Option<u32>,
    /// Exception tag index (WASM EH): defined in tag section, used by throw/try_table
    pub(super) exn_tag_idx: Option<u32>,
    /// Index of imported __nx_capture_backtrace function (called before throw)
    pub(super) capture_bt_func_idx: Option<u32>,
    /// Map from function name to its index in the funcref table
    pub(super) funcref_table_indices: HashMap<Symbol, u32>,
    /// Map from WASM signature key (params+results) to type index for call_indirect
    pub(super) indirect_type_indices: HashMap<String, u32>,
    /// Index of imported __nx_lazy_spawn function
    pub(super) lazy_spawn_func_idx: Option<u32>,
    /// Index of imported __nx_lazy_join function
    pub(super) lazy_join_func_idx: Option<u32>,
}

pub(super) fn build_codegen_layout(program: &LirProgram) -> Result<CodegenLayout, CodegenError> {
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
        next_offset = next_offset
            .checked_add(len)
            .ok_or(CodegenError::StringLiteralsWithoutMemory)?;
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
        allocate_func_idx: None,
        exn_tag_idx: None,
        capture_bt_func_idx: None,
        funcref_table_indices: HashMap::new(),
        indirect_type_indices: HashMap::new(),
        lazy_spawn_func_idx: None,
        lazy_join_func_idx: None,
    })
}

/// Normalize module name for memory sharing: all `nexus:stdlib/*` WIT interfaces
/// share the same underlying stdlib.wasm memory, so they count as one module.
fn normalize_module_for_memory(module: &str) -> &str {
    if module.starts_with("nexus:stdlib/") {
        "nexus:stdlib/*"
    } else {
        module
    }
}

fn choose_memory_mode(
    program: &LirProgram,
    has_string_literals: bool,
    object_heap_enabled: bool,
) -> Result<MemoryMode, CodegenError> {
    let mut modules_with_string_abi = HashSet::new();
    // Pick the first actual module name (not normalized) for the import.
    let mut first_stdlib_module: Option<Symbol> = None;
    for ext in &program.externals {
        if external_uses_string_abi(ext) {
            let normalized = normalize_module_for_memory(ext.wasm_module.as_ref());
            modules_with_string_abi.insert(normalized.to_string());
            if first_stdlib_module.is_none() && normalized == "nexus:stdlib/*" {
                first_stdlib_module = Some(ext.wasm_module.clone());
            }
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
        // Use the actual WIT module name (not the normalized key) for the import.
        let import_module = if module == "nexus:stdlib/*" {
            first_stdlib_module
                .map(|s| s.to_string())
                .unwrap_or(module)
        } else {
            module
        };
        return Ok(MemoryMode::Imported {
            module: import_module,
        });
    }

    if has_string_literals || object_heap_enabled {
        Ok(MemoryMode::Defined)
    } else {
        Ok(MemoryMode::None)
    }
}

pub(super) fn align8(v: u32) -> u32 {
    (v + 7) & !7
}

pub(super) fn program_uses_object_heap(program: &LirProgram) -> bool {
    for func in &program.functions {
        for stmt in &func.body {
            if stmt_uses_object_heap(stmt) {
                return true;
            }
        }
    }
    false
}

fn stmt_uses_object_heap(stmt: &LirStmt) -> bool {
    match stmt {
        LirStmt::Let { expr, .. } => expr_uses_object_heap(expr),
        LirStmt::If {
            then_body,
            else_body,
            ..
        } => {
            then_body.iter().any(stmt_uses_object_heap)
                || else_body.iter().any(stmt_uses_object_heap)
        }
        LirStmt::IfReturn {
            then_body,
            else_body,
            ..
        } => {
            then_body.iter().any(stmt_uses_object_heap)
                || else_body.iter().any(stmt_uses_object_heap)
        }
        LirStmt::TryCatch {
            body, catch_body, ..
        } => body.iter().any(stmt_uses_object_heap) || catch_body.iter().any(stmt_uses_object_heap),
        LirStmt::Loop {
            cond_stmts, body, ..
        } => cond_stmts.iter().any(stmt_uses_object_heap) || body.iter().any(stmt_uses_object_heap),
        LirStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            cases
                .iter()
                .any(|c| c.body.iter().any(stmt_uses_object_heap))
                || default_body.iter().any(stmt_uses_object_heap)
        }
    }
}

fn expr_uses_object_heap(expr: &LirExpr) -> bool {
    matches!(
        expr,
        LirExpr::Constructor { .. }
            | LirExpr::Record { .. }
            | LirExpr::ObjectTag { .. }
            | LirExpr::ObjectField { .. }
            | LirExpr::FuncRef { .. }
            | LirExpr::Closure { .. }
    ) || matches!(
        expr,
        LirExpr::Binary { op, typ, .. } if is_string_concat_operator(*op, typ)
    )
}

fn external_uses_string_abi(ext: &LirExternal) -> bool {
    ext.params
        .iter()
        .any(|p| matches!(peel_linear(&p.typ), Type::String))
        || matches!(peel_linear(&ext.ret_type), Type::String)
}

fn collect_strings_in_stmt(stmt: &LirStmt, out: &mut Vec<String>) {
    match stmt {
        LirStmt::Let { expr, .. } => collect_strings_in_expr(expr, out),
        LirStmt::If {
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
        LirStmt::IfReturn {
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
            if let Some(then_ret) = then_ret {
                collect_strings_in_atom(then_ret, out);
            }
            for stmt in else_body {
                collect_strings_in_stmt(stmt, out);
            }
            if let Some(else_ret) = else_ret {
                collect_strings_in_atom(else_ret, out);
            }
        }
        LirStmt::TryCatch {
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
        LirStmt::Loop {
            cond_stmts,
            cond,
            body,
        } => {
            for stmt in cond_stmts {
                collect_strings_in_stmt(stmt, out);
            }
            collect_strings_in_atom(cond, out);
            for stmt in body {
                collect_strings_in_stmt(stmt, out);
            }
        }
        LirStmt::Switch {
            tag,
            cases,
            default_body,
            default_ret,
            ..
        } => {
            collect_strings_in_atom(tag, out);
            for case in cases {
                for stmt in &case.body {
                    collect_strings_in_stmt(stmt, out);
                }
                if let Some(ret) = &case.ret {
                    collect_strings_in_atom(ret, out);
                }
            }
            for stmt in default_body {
                collect_strings_in_stmt(stmt, out);
            }
            if let Some(ret) = default_ret {
                collect_strings_in_atom(ret, out);
            }
        }
    }
}

fn collect_strings_in_expr(expr: &LirExpr, out: &mut Vec<String>) {
    match expr {
        LirExpr::Atom(atom) => collect_strings_in_atom(atom, out),
        LirExpr::Binary { lhs, rhs, .. } => {
            collect_strings_in_atom(lhs, out);
            collect_strings_in_atom(rhs, out);
        }
        LirExpr::Call { args, .. } | LirExpr::TailCall { args, .. } => {
            for (_, atom) in args {
                collect_strings_in_atom(atom, out);
            }
        }
        LirExpr::Constructor { args, .. } => {
            for atom in args {
                collect_strings_in_atom(atom, out);
            }
        }
        LirExpr::Record { fields, .. } => {
            for (_, atom) in fields {
                collect_strings_in_atom(atom, out);
            }
        }
        LirExpr::ObjectTag { value, .. } => collect_strings_in_atom(value, out),
        LirExpr::ObjectField { value, .. } => collect_strings_in_atom(value, out),
        LirExpr::Raise { value, .. } => collect_strings_in_atom(value, out),
        LirExpr::Force { value, .. } => collect_strings_in_atom(value, out),
        LirExpr::FuncRef { .. } | LirExpr::ClosureEnvLoad { .. } => {}
        LirExpr::Closure { captures, .. } => {
            for (_, atom) in captures {
                collect_strings_in_atom(atom, out);
            }
        }
        LirExpr::CallIndirect { callee, args, .. } => {
            collect_strings_in_atom(callee, out);
            for (_, atom) in args {
                collect_strings_in_atom(atom, out);
            }
        }
        LirExpr::LazySpawn { thunk, .. } => collect_strings_in_atom(thunk, out),
        LirExpr::LazyJoin { task_id, .. } => collect_strings_in_atom(task_id, out),
    }
}

fn collect_strings_in_atom(atom: &LirAtom, out: &mut Vec<String>) {
    if let LirAtom::String(s) = atom {
        out.push(s.clone());
    }
}
