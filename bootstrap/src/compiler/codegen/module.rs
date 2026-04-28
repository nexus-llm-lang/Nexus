use std::collections::{HashMap, HashSet};

use wasm_encoder::{
    CodeSection, ConstExpr, DataSection, ElementSection, Elements, EntityType, ExportKind,
    ExportSection, Function, FunctionSection, GlobalSection, GlobalType, ImportSection,
    Instruction, MemorySection, MemoryType, Module, NameMap, NameSection, RefType, TableSection,
    TableType, TagKind, TagSection, TagType, TypeSection, ValType,
};

use crate::constants::{ENTRYPOINT, MEMORY_EXPORT};
use crate::intern::Symbol;
use crate::ir::lir::{LirExpr, LirExternal, LirProgram, LirStmt};
use crate::types::Type;

use super::dwarf::FuncDebugEntry;
use super::emit::{
    external_param_types, external_return_types, peel_linear, return_type_to_wasm_result,
    type_to_wasm_valtype,
};
use super::error::CodegenError;
use super::function::compile_function;
use super::layout::{build_codegen_layout, program_uses_object_heap, CodegenLayout, MemoryMode};
use super::string::string_abi_for_external;
use super::{
    ALLOCATE_WASM_NAME, ALLOC_MARK_WASM_NAME, ALLOC_RESET_WASM_NAME, BT_CAPTURE_NAME, BT_MODULE,
    LAZY_ALLOC_MARK_NAME, LAZY_ALLOC_NAME, LAZY_ALLOC_RESET_NAME, LAZY_JOIN_NAME, LAZY_MODULE,
    LAZY_SPAWN_NAME, OBJECT_HEAP_GLOBAL_INDEX,
};

/// Compiles LIR (in ANF) directly into core WASM bytes, plus debug entries for DWARF.
pub fn compile_lir_to_wasm(
    program: &LirProgram,
) -> Result<(Vec<u8>, Vec<FuncDebugEntry>), CodegenError> {
    let (wasm, debug, _heap_base) = compile_lir_to_wasm_inner(program, false)?;
    Ok((wasm, debug))
}

/// Threaded variant — emits `(import "env" "memory" (memory N M shared))` in
/// place of a locally defined memory, for use with `LazyRuntime::with_shared_memory`.
/// Internal to the runtime/test path.
pub fn compile_lir_to_wasm_threaded(
    program: &LirProgram,
) -> Result<(Vec<u8>, Vec<FuncDebugEntry>), CodegenError> {
    let (wasm, debug, _heap_base) = compile_lir_to_wasm_inner(program, true)?;
    Ok((wasm, debug))
}

/// Threaded variant that also surfaces the program's `heap_base` so callers
/// can seed `LazyRuntime`'s atomic bump allocator. Test-only.
pub fn compile_lir_to_wasm_threaded_with_heap_base(
    program: &LirProgram,
) -> Result<(Vec<u8>, Vec<FuncDebugEntry>, i32), CodegenError> {
    compile_lir_to_wasm_inner(program, true)
}

fn compile_lir_to_wasm_inner(
    program: &LirProgram,
    memory_shared: bool,
) -> Result<(Vec<u8>, Vec<FuncDebugEntry>, i32), CodegenError> {
    let has_eh = program_needs_eh(program);
    // Only import __nx_capture_backtrace when a catch body actually uses backtrace().
    // This is the "notrace" optimization: most programs use try/catch without inspecting
    // the call stack, so we skip the expensive wasmtime stack walk at every throw.
    let needs_bt_capture = has_eh && program_uses_backtrace(program);
    let needs_lazy = program_needs_lazy(program);

    // With component-model (canonical ABI), each component has its own memory and
    // the user code uses an internal bump allocator for heap operations.
    let stdlib_alloc_module = if program_uses_object_heap(program) {
        program
            .externals
            .iter()
            .find(|ext| {
                is_stdlib_module(ext.wasm_module.as_ref())
                    && string_abi_for_external(ext) == super::string::StringABI::Packed
            })
            .map(|ext| ext.wasm_module.to_string())
    } else {
        None
    };
    // Skip allocate import when arena intrinsics force bump allocator
    // In component model (MemoryMode::Defined), each component has its own memory.
    // stdlib's allocate returns pointers into STDLIB's memory, not the user module's.
    // Only import allocate when sharing memory (wasm-merge bundling).
    let is_component = program.externals.iter().any(|ext| {
        super::string::string_abi_for_external(ext) == super::string::StringABI::Canonical
    }) || program.externals.iter().any(|ext| {
        ext.wasm_module.as_ref().contains(':')
            && !ext.wasm_module.as_ref().starts_with("nexus:runtime/")
    });
    let needs_alloc_import = stdlib_alloc_module.is_some() && !is_component;
    // 3 imports when alloc is needed: allocate, __nx_alloc_mark, __nx_alloc_reset.
    // mark/reset wire up `arena.heap_reset` so it reclaims stdlib-routed string
    // and object allocations alongside the G0 bump pointer (otherwise long-lived
    // programs leak every concat / format / substring through `Call(allocate)`).
    let n_alloc_imports: u32 = if needs_alloc_import { 3 } else { 0 };

    // Deduplicate externals by (wasm_module, wasm_name) — multiple Nexus names
    // pointing to the same underlying WASM function share a single WASM import.
    let mut wasm_import_dedup: HashMap<(Symbol, Symbol), u32> = HashMap::new();
    let mut deduped_externals: Vec<&LirExternal> = Vec::new();
    let mut external_function_indices = HashMap::new();
    for ext in &program.externals {
        // Skip intrinsic-only modules — all functions are inlined, no import needed.
        if ext.wasm_module.as_ref() == "nexus:runtime/arena" {
            continue;
        }
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
    let n_bt_imports: u32 = if needs_bt_capture { 1 } else { 0 };
    // spawn + join, plus alloc / alloc-mark / alloc-reset in shared-memory
    // mode (host-side atomic bump allocator + region-reset hooks — see
    // LazyRuntime::with_shared_memory and LazyRuntime::register).
    let n_lazy_imports: u32 = if needs_lazy {
        if memory_shared {
            5
        } else {
            2
        }
    } else {
        0
    };

    // Check if any external uses canonical ABI (component model boundaries)
    let needs_cabi_realloc = program
        .externals
        .iter()
        .any(|ext| string_abi_for_external(ext) == super::string::StringABI::Canonical);

    let import_count = deduped_ext_count + n_alloc_imports + n_bt_imports + n_lazy_imports;

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

    // Collect funcref targets and indirect call types
    let funcref_targets = collect_funcref_targets(program);
    let indirect_call_types = collect_indirect_call_types(program);
    let has_funcref = !funcref_targets.is_empty();

    let mut layout = build_codegen_layout(program)?;
    layout.memory_shared = memory_shared;
    if needs_alloc_import {
        let alloc_idx = deduped_ext_count;
        layout.allocate_func_idx = Some(alloc_idx);
        layout.alloc_mark_func_idx = Some(alloc_idx + 1);
        layout.alloc_reset_func_idx = Some(alloc_idx + 2);
    }
    if needs_bt_capture {
        let bt_idx = deduped_ext_count + n_alloc_imports;
        layout.capture_bt_func_idx = Some(bt_idx);
    }
    if needs_lazy {
        let lazy_base = deduped_ext_count + n_alloc_imports + n_bt_imports;
        layout.lazy_spawn_func_idx = Some(lazy_base);
        layout.lazy_join_func_idx = Some(lazy_base + 1);
        if memory_shared {
            // Host-side atomic bump allocator. Set allocate_func_idx so
            // every existing `if let Some(alloc_idx) = layout.allocate_func_idx`
            // alloc site routes through it instead of inline GlobalGet/GlobalSet
            // — concurrent allocations from worker thunks then atomically
            // advance a single shared heap pointer instead of racing on
            // per-instance globals.
            layout.allocate_func_idx = Some(lazy_base + 2);
            // Region reset support: the lazy host allocator exposes its own
            // (mark, reset) pair operating directly on the AtomicI32 bump
            // pointer. Wiring them through `arena.heap_reset` means caller
            // and worker allocations alike rewind together — without these,
            // `heap_reset` would silently leak every worker-side allocation
            // (the stdlib mark/reset path tracks a different counter).
            // The packed-i64 dual-tracking that legacy mode uses for stdlib
            // mark/reset is unnecessary here: shared-memory mode has no
            // separate G0 to coordinate with, only the host-side alloc_ptr.
            layout.alloc_mark_func_idx = Some(lazy_base + 3);
            layout.alloc_reset_func_idx = Some(lazy_base + 4);
            layout.lazy_host_arena = true;
        }
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
        let type_idx = if let Some(&existing) = sig_to_type_idx.get(&key) {
            existing
        } else {
            types.ty().function(params, results);
            sig_to_type_idx.insert(key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
        external_type_indices.push(type_idx);
    }

    // Exception tag type: (i64) -> () — the exception payload is a packed i64
    let mut exn_tag_type_idx = 0;
    if has_eh {
        types.ty().function([ValType::I64], []);
        exn_tag_type_idx = next_type_index;
        next_type_index += 1;
    }

    let mut allocate_type_idx = 0;
    let mut alloc_mark_type_idx = 0;
    let mut alloc_reset_type_idx = 0;
    if needs_alloc_import {
        types.ty().function([ValType::I32], [ValType::I32]);
        allocate_type_idx = next_type_index;
        next_type_index += 1;
        // __nx_alloc_mark: () -> i32
        let mark_key = sig_key(&[], &[ValType::I32]);
        alloc_mark_type_idx = if let Some(&existing) = sig_to_type_idx.get(&mark_key) {
            existing
        } else {
            types.ty().function([], [ValType::I32]);
            sig_to_type_idx.insert(mark_key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
        // __nx_alloc_reset: (i32) -> ()
        let reset_key = sig_key(&[ValType::I32], &[]);
        alloc_reset_type_idx = if let Some(&existing) = sig_to_type_idx.get(&reset_key) {
            existing
        } else {
            types.ty().function([ValType::I32], []);
            sig_to_type_idx.insert(reset_key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
    }

    // Backtrace capture type: () -> () — called before throw (only when backtrace is used)
    let mut bt_capture_type_idx = 0;
    if needs_bt_capture {
        let key = sig_key(&[], &[]);
        bt_capture_type_idx = if let Some(&existing) = sig_to_type_idx.get(&key) {
            existing
        } else {
            types.ty().function([], []);
            sig_to_type_idx.insert(key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
    }

    // Lazy spawn type: (i64, i32) -> i64 — spawn a thunk for parallel evaluation
    let mut lazy_spawn_type_idx = 0;
    // Lazy join type: (i64) -> i64 — wait for a spawned thunk result
    let mut lazy_join_type_idx = 0;
    // Lazy alloc type: (i32) -> i32 — atomic bump allocator (shared-memory mode).
    let mut lazy_alloc_type_idx = 0;
    // Lazy alloc-mark type: () -> i32 — snapshot host-side bump pointer.
    let mut lazy_alloc_mark_type_idx = 0;
    // Lazy alloc-reset type: (i32) -> () — restore host-side bump pointer.
    let mut lazy_alloc_reset_type_idx = 0;
    if needs_lazy {
        let spawn_key = sig_key(&[ValType::I64, ValType::I32], &[ValType::I64]);
        lazy_spawn_type_idx = if let Some(&existing) = sig_to_type_idx.get(&spawn_key) {
            existing
        } else {
            types
                .ty()
                .function([ValType::I64, ValType::I32], [ValType::I64]);
            sig_to_type_idx.insert(spawn_key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
        let join_key = sig_key(&[ValType::I64], &[ValType::I64]);
        lazy_join_type_idx = if let Some(&existing) = sig_to_type_idx.get(&join_key) {
            existing
        } else {
            types.ty().function([ValType::I64], [ValType::I64]);
            sig_to_type_idx.insert(join_key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
        if memory_shared {
            let alloc_key = sig_key(&[ValType::I32], &[ValType::I32]);
            lazy_alloc_type_idx = if let Some(&existing) = sig_to_type_idx.get(&alloc_key) {
                existing
            } else {
                types.ty().function([ValType::I32], [ValType::I32]);
                sig_to_type_idx.insert(alloc_key, next_type_index);
                let idx = next_type_index;
                next_type_index += 1;
                idx
            };
            // alloc-mark: () -> i32
            let mark_key = sig_key(&[], &[ValType::I32]);
            lazy_alloc_mark_type_idx = if let Some(&existing) = sig_to_type_idx.get(&mark_key) {
                existing
            } else {
                types.ty().function([], [ValType::I32]);
                sig_to_type_idx.insert(mark_key, next_type_index);
                let idx = next_type_index;
                next_type_index += 1;
                idx
            };
            // alloc-reset: (i32) -> ()
            let reset_key = sig_key(&[ValType::I32], &[]);
            lazy_alloc_reset_type_idx = if let Some(&existing) = sig_to_type_idx.get(&reset_key) {
                existing
            } else {
                types.ty().function([ValType::I32], []);
                sig_to_type_idx.insert(reset_key, next_type_index);
                let idx = next_type_index;
                next_type_index += 1;
                idx
            };
        }
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
        let type_idx = if let Some(&existing) = sig_to_type_idx.get(&key) {
            existing
        } else {
            types.ty().function(params, results);
            sig_to_type_idx.insert(key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
        internal_type_indices.push(type_idx);
    }
    // cabi_realloc type: (old_ptr: i32, old_size: i32, align: i32, new_size: i32) -> i32
    let mut cabi_realloc_type_idx = 0;
    if needs_cabi_realloc {
        let params = vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32];
        let results = vec![ValType::I32];
        let key = sig_key(&params, &results);
        cabi_realloc_type_idx = if let Some(&existing) = sig_to_type_idx.get(&key) {
            existing
        } else {
            types.ty().function(params, results);
            sig_to_type_idx.insert(key, next_type_index);
            let idx = next_type_index;
            next_type_index += 1;
            idx
        };
    }

    let wasi_key = sig_key(&[], &[]);
    let wasi_cli_run_type_index = if let Some(&existing) = sig_to_type_idx.get(&wasi_key) {
        existing
    } else {
        types.ty().function([], []);
        sig_to_type_idx.insert(wasi_key, next_type_index);
        let idx = next_type_index;
        next_type_index += 1;
        idx
    };

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
    if layout.memory_shared {
        // Threaded mode (nexus-tb6p slice 2): host provides a wasmtime::SharedMemory
        // under ("env", "memory"). The lazy runtime's `with_shared_memory`
        // constructor binds the same SharedMemory in worker linkers, so the
        // user wasm and every worker share a single linear memory.
        imports.import(
            "env",
            MEMORY_EXPORT,
            EntityType::Memory(MemoryType {
                minimum: 1,
                maximum: Some(65536),
                memory64: false,
                shared: true,
                page_size_log2: None,
            }),
        );
        has_imports = true;
    } else if let MemoryMode::Imported { module: mem_module } = &layout.memory_mode {
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
    if needs_alloc_import {
        if let Some(alloc_module) = &stdlib_alloc_module {
            imports.import(
                alloc_module,
                ALLOCATE_WASM_NAME,
                EntityType::Function(allocate_type_idx),
            );
            imports.import(
                alloc_module,
                ALLOC_MARK_WASM_NAME,
                EntityType::Function(alloc_mark_type_idx),
            );
            imports.import(
                alloc_module,
                ALLOC_RESET_WASM_NAME,
                EntityType::Function(alloc_reset_type_idx),
            );
            has_imports = true;
        }
    }
    if needs_bt_capture {
        imports.import(
            BT_MODULE,
            BT_CAPTURE_NAME,
            EntityType::Function(bt_capture_type_idx),
        );
        has_imports = true;
    }
    if needs_lazy {
        imports.import(
            LAZY_MODULE,
            LAZY_SPAWN_NAME,
            EntityType::Function(lazy_spawn_type_idx),
        );
        imports.import(
            LAZY_MODULE,
            LAZY_JOIN_NAME,
            EntityType::Function(lazy_join_type_idx),
        );
        if memory_shared {
            // Atomic bump allocator (host-side AtomicI32) — see
            // LazyRuntime::with_shared_memory. Order matters: 3rd / 4th /
            // 5th lazy imports so their function indices are
            // `lazy_base + 2..=4`, matching `layout.allocate_func_idx`,
            // `alloc_mark_func_idx`, `alloc_reset_func_idx` set above.
            imports.import(
                LAZY_MODULE,
                LAZY_ALLOC_NAME,
                EntityType::Function(lazy_alloc_type_idx),
            );
            imports.import(
                LAZY_MODULE,
                LAZY_ALLOC_MARK_NAME,
                EntityType::Function(lazy_alloc_mark_type_idx),
            );
            imports.import(
                LAZY_MODULE,
                LAZY_ALLOC_RESET_NAME,
                EntityType::Function(lazy_alloc_reset_type_idx),
            );
        }
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
    if needs_cabi_realloc {
        functions.function(cabi_realloc_type_idx);
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
    // In threaded mode the memory is host-provided via the import section
    // above; emitting a local memory section too would cause two memories.
    if matches!(layout.memory_mode, MemoryMode::Defined) && !layout.memory_shared {
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 256, // 16MB initial — grows via memory.grow as needed
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
        // Emit bump-allocator global when object heap is active without stdlib
        // allocator, OR when cabi_realloc needs a fallback allocator.
        // Also emit when there are string literals (G1 = str_base needed for addressing).
        let needs_heap_global = ((layout.object_heap_enabled || needs_cabi_realloc)
            && layout.allocate_func_idx.is_none())
            || !layout.data_segments.is_empty();
        if needs_heap_global {
            let mut globals = GlobalSection::new();
            // Global 0: object heap pointer (for constructors, string concat, retptr)
            globals.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(layout.heap_base as i32),
            );
            // Global 1: placeholder. Kept for global index stability.
            globals.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(0),
            );
            // Global 2: placeholder. Kept for global index stability.
            globals.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(0),
            );
            module.section(&globals);
        }
    }

    // === Export Section ===
    let mut exports = ExportSection::new();
    exports.export(ENTRYPOINT, ExportKind::Func, main_idx);
    let n_cabi_realloc: u32 = if needs_cabi_realloc { 1 } else { 0 };
    let cabi_realloc_func_idx = import_count + program.functions.len() as u32;
    let wasi_cli_run_func_idx = cabi_realloc_func_idx + n_cabi_realloc;
    // _start: WASI P1 entry point for wasmtime core module execution
    exports.export("_start", ExportKind::Func, wasi_cli_run_func_idx);
    if needs_cabi_realloc {
        exports.export("cabi_realloc", ExportKind::Func, cabi_realloc_func_idx);
    }
    if !matches!(layout.memory_mode, MemoryMode::None) {
        exports.export(MEMORY_EXPORT, ExportKind::Memory, 0);
    }
    // Export the indirect function table when present so host runtime
    // functions (__nx_lazy_spawn and friends) can call_indirect thunks.
    if has_funcref {
        exports.export("__indirect_function_table", ExportKind::Table, 0);
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
    let mut debug_entries = Vec::new();
    // The code section body starts with a LEB128 function count.
    // +1 for WASI CLI run wrapper, +1 for cabi_realloc if needed
    let total_func_count = program.functions.len() + (if needs_cabi_realloc { 1 } else { 0 }) + 1;
    let mut code_body_offset = uleb128_encoded_size(total_func_count as u64) as u32;
    for func in &program.functions {
        let body = compile_function(
            func,
            program,
            &internal_function_indices,
            &external_function_indices,
            &layout,
        )?;
        let body_byte_len = body.byte_len();
        // Each function entry is LEB128(body_size) + body_bytes
        let func_encoded_size =
            uleb128_encoded_size(body_byte_len as u64) as u32 + body_byte_len as u32;
        debug_entries.push(FuncDebugEntry {
            name: func.name.to_string(),
            code_offset: code_body_offset,
            code_size: func_encoded_size,
            source_file: func.source_file.clone(),
            source_line: func.source_line,
        });
        code_body_offset += func_encoded_size;
        code.function(&body);
    }
    if needs_cabi_realloc {
        let cabi_realloc_body = compile_cabi_realloc(&layout);
        code.function(&cabi_realloc_body);
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
        if needs_alloc_import {
            func_names.append(idx, ALLOCATE_WASM_NAME);
            idx += 1;
            func_names.append(idx, ALLOC_MARK_WASM_NAME);
            idx += 1;
            func_names.append(idx, ALLOC_RESET_WASM_NAME);
            idx += 1;
        }
        if needs_bt_capture {
            func_names.append(idx, BT_CAPTURE_NAME);
            idx += 1;
        }
        if needs_lazy {
            func_names.append(idx, LAZY_SPAWN_NAME);
            idx += 1;
            func_names.append(idx, LAZY_JOIN_NAME);
            idx += 1;
            if memory_shared {
                func_names.append(idx, LAZY_ALLOC_NAME);
                idx += 1;
                func_names.append(idx, LAZY_ALLOC_MARK_NAME);
                idx += 1;
                func_names.append(idx, LAZY_ALLOC_RESET_NAME);
                idx += 1;
            }
        }
        // Internal functions (indices import_count..import_count+n_funcs)
        for func in &program.functions {
            func_names.append(idx, func.name.as_str());
            idx += 1;
        }
        // cabi_realloc (canonical ABI allocator)
        if needs_cabi_realloc {
            func_names.append(idx, "cabi_realloc");
            idx += 1;
        }
        // WASI P1 _start wrapper
        func_names.append(idx, "_start");
        names.functions(&func_names);
        module.section(&names);
    }

    Ok((module.finish(), debug_entries, layout.heap_base as i32))
}

fn compile_wasi_cli_run_wrapper(main_idx: u32, main_ret_type: &Type) -> Function {
    let mut body = Function::new(Vec::new());
    body.instruction(&Instruction::Call(main_idx));
    if !matches!(peel_linear(main_ret_type), Type::Unit) {
        body.instruction(&Instruction::Drop);
    }
    body.instruction(&Instruction::End);
    body
}

/// Generate a `cabi_realloc` function for the canonical ABI.
///
/// Signature: `(old_ptr: i32, old_size: i32, align: i32, new_size: i32) -> i32`
///
/// Delegates to stdlib `allocate` if available, otherwise bumps G0 (shared with
/// the object heap). The shared approach is safe as long as G0 grows monotonically
/// (no heap_reset while cabi strings are still live).
fn compile_cabi_realloc(layout: &CodegenLayout) -> Function {
    // Locals: params are 0=old_ptr, 1=old_size, 2=align, 3=new_size
    // Extra locals: 4=aligned_ptr
    let mut body = Function::new(vec![(1, ValType::I32)]);
    let aligned_ptr_local = 4u32;
    let has_memory = matches!(layout.memory_mode, MemoryMode::Defined);

    if let Some(alloc_idx) = layout.allocate_func_idx {
        body.instruction(&Instruction::LocalGet(3));
        body.instruction(&Instruction::Call(alloc_idx));
    } else {
        // Bump on object heap (global 0), shared with object allocation.
        // Both advance G0 monotonically — no collision as long as heap_reset
        // is not called while cabi strings are still live.
        body.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
        body.instruction(&Instruction::LocalGet(2)); // align
        body.instruction(&Instruction::I32Add);
        body.instruction(&Instruction::I32Const(1));
        body.instruction(&Instruction::I32Sub);
        body.instruction(&Instruction::I32Const(0));
        body.instruction(&Instruction::LocalGet(2)); // align
        body.instruction(&Instruction::I32Sub);
        body.instruction(&Instruction::I32And);
        body.instruction(&Instruction::LocalSet(aligned_ptr_local));

        // Advance heap pointer
        body.instruction(&Instruction::LocalGet(aligned_ptr_local));
        body.instruction(&Instruction::LocalGet(3)); // new_size
        body.instruction(&Instruction::I32Add);
        body.instruction(&Instruction::GlobalSet(OBJECT_HEAP_GLOBAL_INDEX));

        if has_memory {
            body.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
            body.instruction(&Instruction::MemorySize(0));
            body.instruction(&Instruction::I32Const(16));
            body.instruction(&Instruction::I32Shl);
            body.instruction(&Instruction::I32GtU);
            body.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
            {
                body.instruction(&Instruction::GlobalGet(OBJECT_HEAP_GLOBAL_INDEX));
                body.instruction(&Instruction::MemorySize(0));
                body.instruction(&Instruction::I32Const(16));
                body.instruction(&Instruction::I32Shl);
                body.instruction(&Instruction::I32Sub);
                body.instruction(&Instruction::I32Const(65535));
                body.instruction(&Instruction::I32Add);
                body.instruction(&Instruction::I32Const(16));
                body.instruction(&Instruction::I32ShrU);
                body.instruction(&Instruction::MemoryGrow(0));
                body.instruction(&Instruction::Drop);
            }
            body.instruction(&Instruction::End);
        }

        // Return aligned_ptr
        body.instruction(&Instruction::LocalGet(aligned_ptr_local));
    }

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
            | LirExpr::Raise { value, .. }
            | LirExpr::Force { value, .. } => {
                scan_atom(value, targets);
            }
            LirExpr::ClosureEnvLoad { .. } => {}
            LirExpr::LazySpawn { thunk, .. } => scan_atom(thunk, targets),
            LirExpr::LazyJoin { task_id, .. } => scan_atom(task_id, targets),
            LirExpr::Intrinsic { args, .. } => {
                for (_, a) in args {
                    scan_atom(a, targets);
                }
            }
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
            LirStmt::FieldUpdate { .. } => {}
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
            LirStmt::FieldUpdate { .. } => {}
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

/// Compute the encoded size of a ULEB128 value.
fn uleb128_encoded_size(mut val: u64) -> usize {
    let mut size = 0;
    loop {
        val >>= 7;
        size += 1;
        if val == 0 {
            break;
        }
    }
    size
}

/// Check if a WASM module name refers to a registered package's interface.
/// Matches both legacy file-path (`nxlib/stdlib/stdlib.wasm`) and
/// WIT-style (`nexus:<pkg>/<iface>`) names.
fn is_stdlib_module(module: &str) -> bool {
    module.ends_with("stdlib.wasm") || crate::lang::stdlib::is_package_wit_module(module)
}

/// Check if any catch body in the program uses the backtrace runtime
/// (i.e., the program imports `__nx_bt_depth` or `__nx_bt_frame`).
/// When false, `__nx_capture_backtrace` can be elided from raise sites
/// because no catch handler ever inspects the captured stack frames.
fn program_uses_backtrace(program: &LirProgram) -> bool {
    program.externals.iter().any(|ext| {
        let m = ext.wasm_module.as_ref();
        m == "nexus:runtime/backtrace"
    })
}

/// Check if the LIR program needs exception handling (throw/catch).
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
            LirStmt::FieldUpdate { .. } => false,
        }
    }
    program
        .functions
        .iter()
        .any(|f| f.body.iter().any(stmt_needs_bt))
}

/// Check if the LIR program contains lazy spawn/join expressions.
fn program_needs_lazy(program: &LirProgram) -> bool {
    fn expr_has_lazy(expr: &LirExpr) -> bool {
        matches!(expr, LirExpr::LazySpawn { .. } | LirExpr::LazyJoin { .. })
    }
    fn stmt_has_lazy(stmt: &LirStmt) -> bool {
        match stmt {
            LirStmt::Let { expr, .. } => expr_has_lazy(expr),
            LirStmt::If {
                then_body,
                else_body,
                ..
            } => then_body.iter().any(stmt_has_lazy) || else_body.iter().any(stmt_has_lazy),
            LirStmt::IfReturn {
                then_body,
                else_body,
                ..
            } => then_body.iter().any(stmt_has_lazy) || else_body.iter().any(stmt_has_lazy),
            LirStmt::Loop {
                cond_stmts, body, ..
            } => cond_stmts.iter().any(stmt_has_lazy) || body.iter().any(stmt_has_lazy),
            LirStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                cases.iter().any(|c| c.body.iter().any(stmt_has_lazy))
                    || default_body.iter().any(stmt_has_lazy)
            }
            LirStmt::TryCatch {
                body, catch_body, ..
            } => body.iter().any(stmt_has_lazy) || catch_body.iter().any(stmt_has_lazy),
            LirStmt::FieldUpdate { .. } => false,
        }
    }
    program
        .functions
        .iter()
        .any(|f| f.body.iter().any(stmt_has_lazy))
}
