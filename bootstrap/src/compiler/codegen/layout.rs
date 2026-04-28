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
    /// Index of imported `__nx_alloc_mark` (paired with `allocate`).
    /// Returns the snapshot of outstanding allocations, packed into the upper
    /// 32 bits of `arena.heap_mark`'s i64 return so `heap_reset` can free them
    /// together with the G0 bump. Set whenever `allocate_func_idx` is set —
    /// otherwise heap_reset would silently leak strings from the stdlib path.
    pub(super) alloc_mark_func_idx: Option<u32>,
    /// Index of imported `__nx_alloc_reset(mark)`. LIFO-frees every
    /// allocation made after `mark`. Called from `arena.heap_reset`.
    pub(super) alloc_reset_func_idx: Option<u32>,
    /// Exception tag index (WASM EH): defined in tag section, used by throw/try_table
    pub(super) exn_tag_idx: Option<u32>,
    /// Index of imported __nx_capture_backtrace function (called before throw)
    pub(super) capture_bt_func_idx: Option<u32>,
    /// Map from function name to its index in the funcref table
    pub(super) funcref_table_indices: HashMap<Symbol, u32>,
    /// Map from WASM signature key (params+results) to type index for call_indirect
    pub(super) indirect_type_indices: HashMap<String, u32>,
    /// Static addresses for nullary constructors (pre-allocated in data section).
    pub(super) nullary_ctor_addrs: HashMap<i64, u32>,
    /// Index of imported __nx_lazy_spawn function
    pub(super) lazy_spawn_func_idx: Option<u32>,
    /// Index of imported __nx_lazy_join function
    pub(super) lazy_join_func_idx: Option<u32>,
    /// When true, emit memory as `shared` so the host can hand a
    /// `wasmtime::SharedMemory` to the caller and to every `LazyRuntime`
    /// worker — capture-bearing thunks then read their captures across
    /// threads. Switches the program's memory from a `MemoryMode::Defined`
    /// (or legacy `Imported`) to a host-imported shared memory under
    /// `(import "env" "memory" (memory N M shared))`. Off by default;
    /// enabled only via the test-only `compile_lir_to_wasm_threaded` path.
    pub(super) memory_shared: bool,
    /// True when `alloc_mark_func_idx` / `alloc_reset_func_idx` point at
    /// `nexus:runtime/lazy::alloc-mark` / `alloc-reset` (the host-side
    /// AtomicI32 region hooks) rather than at the stdlib's
    /// `__nx_alloc_mark` / `__nx_alloc_reset`. The two pairs have different
    /// signatures and semantics: stdlib mark returns an outstanding
    /// allocation count packed into the upper 32 bits of `arena.heap_mark`
    /// alongside G0; lazy mark returns the bump pointer directly because
    /// shared-memory mode has no separate G0 to coordinate with. Codegen
    /// of `Intrinsic::HeapMark` / `Intrinsic::HeapReset` consults this
    /// flag to pick the correct packing.
    pub(super) lazy_host_arena: bool,
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

    // Collect nullary constructors and pre-allocate static slots (8 bytes each).
    let mut nullary_ctors = HashSet::new();
    for func in &program.functions {
        for stmt in &func.body {
            collect_nullary_ctors_in_stmt(stmt, &mut nullary_ctors);
        }
    }
    let mut nullary_ctor_addrs = HashMap::new();
    // Align to 8 bytes before nullary ctor slots
    next_offset = align8(next_offset);
    for tag in &nullary_ctors {
        nullary_ctor_addrs.insert(*tag, next_offset);
        data_segments.push(DataSegment {
            offset: next_offset,
            bytes: tag.to_le_bytes().to_vec(),
        });
        next_offset += 8;
    }

    let heap_base = align8(next_offset.max(STRING_DATA_BASE));

    Ok(CodegenLayout {
        nullary_ctor_addrs,
        memory_mode,
        string_literals: literal_map,
        data_segments,
        object_heap_enabled,
        heap_base,
        allocate_func_idx: None,
        alloc_mark_func_idx: None,
        alloc_reset_func_idx: None,
        exn_tag_idx: None,
        capture_bt_func_idx: None,
        funcref_table_indices: HashMap::new(),
        indirect_type_indices: HashMap::new(),
        lazy_spawn_func_idx: None,
        lazy_join_func_idx: None,
        memory_shared: false,
        lazy_host_arena: false,
    })
}

/// Normalize module name for memory sharing.
/// Component-model modules (containing ':') each own their memory — not shared.
/// Only file-path imports (shared-memory bundling) share memory across modules.
fn normalize_module_for_memory(module: &str) -> &str {
    // Component-model modules (nexus:std/*, nexus:cli/*, etc.) are separate
    // components with their own linear memory. They don't share memory.
    // Only non-WIT file-path imports (legacy shared-memory bundling) share.
    module
}

fn choose_memory_mode(
    program: &LirProgram,
    has_string_literals: bool,
    object_heap_enabled: bool,
) -> Result<MemoryMode, CodegenError> {
    // Component-model modules (containing ':') each own their memory.
    // Only non-WIT file-path imports support shared memory via MemoryMode::Imported.
    let mut shared_memory_modules = HashSet::new();
    for ext in &program.externals {
        if external_uses_string_abi(ext) {
            let module = ext.wasm_module.as_ref();
            // Component-model modules don't share memory — skip them.
            if module.contains(':') {
                continue;
            }
            let normalized = normalize_module_for_memory(module);
            shared_memory_modules.insert(normalized.to_string());
        }
    }

    // If there's exactly one shared-memory module, import its memory.
    if shared_memory_modules.len() == 1 {
        let module = shared_memory_modules.into_iter().next().unwrap();
        return Ok(MemoryMode::Imported { module });
    }

    // Multiple shared-memory modules, or component-model with string use, or
    // string literals, or object heap → define our own memory.
    let needs_memory = has_string_literals
        || object_heap_enabled
        || program.externals.iter().any(|ext| {
            let m = ext.wasm_module.as_ref();
            m.contains(':') && external_uses_string_abi(ext)
        });
    if needs_memory {
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
        LirStmt::FieldUpdate { .. } => true,
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
        LirStmt::FieldUpdate { .. } => {}
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
        LirExpr::Intrinsic { args, .. } => {
            for (_, a) in args {
                collect_strings_in_atom(a, out);
            }
        }
    }
}

fn collect_strings_in_atom(atom: &LirAtom, out: &mut Vec<String>) {
    if let LirAtom::String(s) = atom {
        out.push(s.clone());
    }
}

use super::emit::constructor_tag;

fn collect_nullary_ctors_in_stmt(stmt: &LirStmt, out: &mut HashSet<i64>) {
    match stmt {
        LirStmt::Let { expr, .. } => collect_nullary_ctors_in_expr(expr, out),
        LirStmt::If {
            then_body,
            else_body,
            ..
        }
        | LirStmt::IfReturn {
            then_body,
            else_body,
            ..
        } => {
            for s in then_body {
                collect_nullary_ctors_in_stmt(s, out);
            }
            for s in else_body {
                collect_nullary_ctors_in_stmt(s, out);
            }
        }
        LirStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for case in cases {
                for s in &case.body {
                    collect_nullary_ctors_in_stmt(s, out);
                }
            }
            for s in default_body {
                collect_nullary_ctors_in_stmt(s, out);
            }
        }
        _ => {}
    }
}

fn collect_nullary_ctors_in_expr(expr: &LirExpr, out: &mut HashSet<i64>) {
    if let LirExpr::Constructor { name, args, .. } = expr {
        if args.is_empty() {
            out.insert(constructor_tag(name.as_str(), 0));
        }
    }
}
