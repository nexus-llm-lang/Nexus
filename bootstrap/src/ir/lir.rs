//! Low-level IR in A-normal form (ANF) — all sub-expressions are atoms.
//! This is the final IR before WASM codegen; codegen consumes LIR directly.

use crate::intern::Symbol;
use crate::types::{BinaryOp, Span, Type};

#[derive(Debug, Clone, PartialEq)]
pub struct LirProgram {
    pub functions: Vec<LirFunction>,
    pub externals: Vec<LirExternal>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LirExternal {
    pub name: Symbol,
    pub wasm_module: Symbol,
    pub wasm_name: Symbol,
    pub params: Vec<LirParam>,
    pub ret_type: Type,
    pub throws: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LirFunction {
    pub name: Symbol,
    pub params: Vec<LirParam>,
    pub ret_type: Type,
    pub requires: Type,
    pub throws: Type,
    pub body: Vec<LirStmt>,
    pub ret: LirAtom,
    pub span: Span,
    pub source_file: Option<String>,
    pub source_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LirParam {
    pub label: Symbol,
    pub name: Symbol,
    pub typ: Type,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LirStmt {
    Let {
        name: Symbol,
        typ: Type,
        expr: LirExpr,
    },
    If {
        cond: LirAtom,
        then_body: Vec<LirStmt>,
        else_body: Vec<LirStmt>,
    },
    IfReturn {
        cond: LirAtom,
        then_body: Vec<LirStmt>,
        /// When Some, emits WASM `return` with this value after the body.
        /// When None, the body executes for side effects only (no WASM return).
        then_ret: Option<LirAtom>,
        else_body: Vec<LirStmt>,
        else_ret: Option<LirAtom>,
        ret_type: Type,
    },
    TryCatch {
        body: Vec<LirStmt>,
        body_ret: Option<LirAtom>,
        catch_param: Symbol,
        catch_param_typ: Type,
        catch_body: Vec<LirStmt>,
        catch_ret: Option<LirAtom>,
    },
    /// Loop with condition check at the top.
    /// cond_stmts compute the break condition, then cond is checked.
    /// If cond is true, break. Otherwise, execute body and repeat.
    Loop {
        cond_stmts: Vec<LirStmt>,
        cond: LirAtom,
        body: Vec<LirStmt>,
    },
    /// Tag-based multi-way branch — compiled to WASM br_table when tags form
    /// a dense integer range, otherwise falls back to a linear if-else chain.
    /// Produced by the LIR optimization pass from IfReturn chains.
    Switch {
        /// The tag atom to dispatch on (typically an ObjectTag result).
        tag: LirAtom,
        /// Cases with known tag values.
        cases: Vec<SwitchCase>,
        /// Default case body (wildcard/variable pattern or last exhaustive case).
        default_body: Vec<LirStmt>,
        default_ret: Option<LirAtom>,
        /// Return type of the overall match.
        ret_type: Type,
    },
    /// In-place heap word update for linear value reuse. Instead of allocating a
    /// new Constructor, overwrites a word in the source object's existing heap memory.
    /// Only emitted when the source is provably dead after this point (no aliases).
    FieldUpdate {
        /// The heap object to update (i64 pointer).
        target: LirAtom,
        /// Byte offset within the heap object (0 = tag, 8 = field 0, 16 = field 1, etc.).
        byte_offset: u64,
        /// New value to store.
        value: LirAtom,
        /// Type of the value (determines the WASM store instruction).
        value_typ: Type,
    },
}

/// A single case in a Switch statement (tag-based multi-way branch).
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchCase {
    pub tag_value: i64,
    pub body: Vec<LirStmt>,
    pub ret: Option<LirAtom>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LirExpr {
    Atom(LirAtom),
    Binary {
        op: BinaryOp,
        lhs: LirAtom,
        rhs: LirAtom,
        typ: Type,
    },
    Call {
        func: Symbol,
        args: Vec<(Symbol, LirAtom)>,
        typ: Type,
    },
    TailCall {
        func: Symbol,
        args: Vec<(Symbol, LirAtom)>,
        typ: Type,
    },
    Constructor {
        name: Symbol,
        args: Vec<LirAtom>,
        typ: Type,
    },
    Record {
        fields: Vec<(Symbol, LirAtom)>,
        typ: Type,
    },
    ObjectTag {
        value: LirAtom,
        typ: Type,
    },
    ObjectField {
        value: LirAtom,
        index: usize,
        typ: Type,
    },
    Raise {
        value: LirAtom,
        typ: Type,
    },
    Force {
        value: LirAtom,
        typ: Type,
    },
    /// Reference to a function as a first-class value (closure pointer as i64)
    FuncRef {
        func: Symbol,
        typ: Type,
    },
    /// Closure: function reference with captured values (closure pointer as i64)
    Closure {
        func: Symbol,
        captures: Vec<(Symbol, LirAtom)>,
        typ: Type,
    },
    /// Load a captured variable from the __env closure pointer.
    /// index 0 = first capture (stored at __env offset 8), etc.
    ClosureEnvLoad {
        index: usize,
        typ: Type,
    },
    /// Indirect call through a funcref value
    CallIndirect {
        callee: LirAtom,
        args: Vec<(Symbol, LirAtom)>,
        typ: Type,
        callee_type: Type,
    },
    /// Spawn a lazy thunk for parallel evaluation.
    /// Calls __nx_lazy_spawn(thunk_ptr, num_captures) -> task_id.
    LazySpawn {
        thunk: LirAtom,
        num_captures: u32,
        typ: Type,
    },
    /// Join a spawned lazy thunk, blocking until the result is available.
    /// Calls __nx_lazy_join(task_id) -> result_value.
    LazyJoin {
        task_id: LirAtom,
        typ: Type,
    },
    /// Inline intrinsic — emitted as direct WASM instructions, no cross-component call.
    Intrinsic {
        kind: Intrinsic,
        args: Vec<(Symbol, LirAtom)>,
        typ: Type,
    },
}

/// Built-in operations emitted as inline WASM instead of external calls.
/// Recognized during LIR lowering from specific (wasm_module, wasm_name) pairs.
#[derive(Debug, Clone, PartialEq)]
pub enum Intrinsic {
    /// string-byte-at(s, idx) → i32.load8_u(ptr + idx)
    StringByteAt,
    /// string-byte-length(s) → packed & 0xFFFFFFFF
    StringByteLength,
    /// skip-ws(s, start) → scan forward while whitespace (0x20,0x09,0x0A,0x0D)
    SkipWs,
    /// scan-ident(s, start) → scan forward while [a-zA-Z0-9_]
    ScanIdent,
    /// scan-digits(s, start) → scan forward while [0-9]
    ScanDigits,
    /// find-byte(s, start, ch) → first index of byte ch, or -1
    FindByte,
    /// byte-substring(s, start, len) → new packed string from byte range
    ByteSubstring,
    /// count-newlines-in(s, start, end_pos) → count of 0x0A in [start, end)
    CountNewlinesIn,
    /// last-newline-in(s, start, end_pos) → last index of 0x0A in [start, end), or -1
    LastNewlineIn,
    /// length(s) → character count (== byte count for ASCII)
    StringLength,
    /// char-code(s, idx) → codepoint at char index as i64 (== byte_at for ASCII)
    CharCode,
    /// char-at(s, idx) → char at char index as i32 (== byte_at for ASCII)
    CharAt,
    /// from-char-code(code) → single-char string from codepoint
    FromCharCode,
    /// from-char(c) → single-char string from char value
    FromChar,
    /// char-ord(c) → char as i64
    CharOrd,
    /// starts-with(s, prefix) → bool: byte-by-byte prefix comparison
    StartsWith,
    /// ends-with(s, suffix) → bool: byte-by-byte suffix comparison
    EndsWith,
    /// contains(s, sub) → bool: scan for substring
    Contains,
    /// index-of(s, sub) → i64: first occurrence of sub, or -1
    IndexOf,
    /// from-i64(val) → string: integer to decimal string representation
    FromI64,
    /// heap-mark() → i64: snapshot object heap pointer for later reset
    HeapMark,
    /// heap-reset(mark: i64) → unit: restore object heap pointer, freeing temp objects
    HeapReset,
    /// heap-swap(base: i64) → i64: swap object heap pointer with a new base, returns old value
    HeapSwap,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LirAtom {
    Var { name: Symbol, typ: Type },
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(char),
    String(String),
    Unit,
}

impl LirAtom {
    pub fn typ(&self) -> Type {
        match self {
            LirAtom::Var { typ, .. } => typ.clone(),
            LirAtom::Int(_) => Type::I64,
            LirAtom::Float(_) => Type::F64,
            LirAtom::Bool(_) => Type::Bool,
            LirAtom::Char(_) => Type::Char,
            LirAtom::String(_) => Type::String,
            LirAtom::Unit => Type::Unit,
        }
    }
}
