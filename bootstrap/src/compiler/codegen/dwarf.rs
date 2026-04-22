//! DWARF v4 debug section emission for WASM.
//!
//! Emits `.debug_abbrev`, `.debug_info`, and `.debug_line` custom sections
//! so that `wasm-tools dump -g` and wasmtime can display source locations.
//!
//! Current granularity: **function-level** — each function maps its entire
//! code range to a single source line. Statement-level precision requires
//! span propagation through MIR/LIR (nexus-cr7).

use std::borrow::Cow;
use std::collections::HashMap;

use wasm_encoder::{CustomSection, Module};

/// Debug info for a single compiled function.
#[derive(Debug, Clone)]
pub struct FuncDebugEntry {
    /// Function name (for DW_AT_name)
    pub name: String,
    /// Byte offset from the start of the code section body
    pub code_offset: u32,
    /// Encoded size of this function in the code section (LEB-prefixed body)
    pub code_size: u32,
    /// Source file path (for DW_AT_decl_file / line program)
    pub source_file: Option<String>,
    /// Source line number, 1-based (for DW_AT_decl_line / line program)
    pub source_line: Option<u32>,
}

// ─── DWARF v4 Constants ────────────────────────────────────────────────────

// Tags
const DW_TAG_COMPILE_UNIT: u8 = 0x11;
const DW_TAG_SUBPROGRAM: u8 = 0x2e;

// Children
const DW_CHILDREN_YES: u8 = 0x01;
const DW_CHILDREN_NO: u8 = 0x00;

// Attributes
const DW_AT_NAME: u8 = 0x03;
const DW_AT_STMT_LIST: u8 = 0x10;
const DW_AT_LOW_PC: u8 = 0x11;
const DW_AT_HIGH_PC: u8 = 0x12;
const DW_AT_LANGUAGE: u8 = 0x13;
const DW_AT_COMP_DIR: u8 = 0x1b;
const DW_AT_DECL_FILE: u8 = 0x3a;
const DW_AT_DECL_LINE: u8 = 0x3b;

// Forms
const DW_FORM_ADDR: u8 = 0x01;
const DW_FORM_DATA4: u8 = 0x06;
const DW_FORM_STRING: u8 = 0x08;
const DW_FORM_DATA1: u8 = 0x0b;
const DW_FORM_UDATA: u8 = 0x0f;

// Line number program opcodes
const DW_LNS_COPY: u8 = 0x01;
const DW_LNS_ADVANCE_PC: u8 = 0x02;
const DW_LNS_ADVANCE_LINE: u8 = 0x03;
const DW_LNS_SET_FILE: u8 = 0x04;
const DW_LNE_END_SEQUENCE: u8 = 0x01;
const DW_LNE_SET_ADDRESS: u8 = 0x02;

// Language
const DW_LANG_LO_USER: u16 = 0x8000;

// ─── LEB128 Helpers ────────────────────────────────────────────────────────

fn write_uleb128(buf: &mut Vec<u8>, mut val: u64) {
    loop {
        let byte = (val & 0x7f) as u8;
        val >>= 7;
        if val == 0 {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    }
}

fn write_sleb128(buf: &mut Vec<u8>, mut val: i64) {
    loop {
        let byte = (val & 0x7f) as u8;
        val >>= 7;
        let done = (val == 0 && byte & 0x40 == 0) || (val == -1 && byte & 0x40 != 0);
        if done {
            buf.push(byte);
            break;
        }
        buf.push(byte | 0x80);
    }
}

fn write_string(buf: &mut Vec<u8>, s: &str) {
    buf.extend_from_slice(s.as_bytes());
    buf.push(0); // null terminator
}

// ─── .debug_abbrev ─────────────────────────────────────────────────────────

fn build_debug_abbrev() -> Vec<u8> {
    let mut buf = Vec::new();

    // Abbrev 1: DW_TAG_compile_unit (has children)
    write_uleb128(&mut buf, 1); // abbrev code
    write_uleb128(&mut buf, DW_TAG_COMPILE_UNIT as u64);
    buf.push(DW_CHILDREN_YES);
    // attributes: name, comp_dir, language, low_pc, high_pc, stmt_list
    write_uleb128(&mut buf, DW_AT_NAME as u64);
    write_uleb128(&mut buf, DW_FORM_STRING as u64);
    write_uleb128(&mut buf, DW_AT_COMP_DIR as u64);
    write_uleb128(&mut buf, DW_FORM_STRING as u64);
    write_uleb128(&mut buf, DW_AT_LANGUAGE as u64);
    write_uleb128(&mut buf, DW_FORM_DATA1 as u64);
    write_uleb128(&mut buf, DW_AT_LOW_PC as u64);
    write_uleb128(&mut buf, DW_FORM_ADDR as u64);
    write_uleb128(&mut buf, DW_AT_HIGH_PC as u64);
    write_uleb128(&mut buf, DW_FORM_DATA4 as u64);
    write_uleb128(&mut buf, DW_AT_STMT_LIST as u64);
    write_uleb128(&mut buf, DW_FORM_DATA4 as u64);
    buf.push(0);
    buf.push(0); // end of attribute list

    // Abbrev 2: DW_TAG_subprogram (no children)
    write_uleb128(&mut buf, 2); // abbrev code
    write_uleb128(&mut buf, DW_TAG_SUBPROGRAM as u64);
    buf.push(DW_CHILDREN_NO);
    // attributes: name, low_pc, high_pc, decl_file, decl_line
    write_uleb128(&mut buf, DW_AT_NAME as u64);
    write_uleb128(&mut buf, DW_FORM_STRING as u64);
    write_uleb128(&mut buf, DW_AT_LOW_PC as u64);
    write_uleb128(&mut buf, DW_FORM_ADDR as u64);
    write_uleb128(&mut buf, DW_AT_HIGH_PC as u64);
    write_uleb128(&mut buf, DW_FORM_DATA4 as u64);
    write_uleb128(&mut buf, DW_AT_DECL_FILE as u64);
    write_uleb128(&mut buf, DW_FORM_UDATA as u64);
    write_uleb128(&mut buf, DW_AT_DECL_LINE as u64);
    write_uleb128(&mut buf, DW_FORM_UDATA as u64);
    buf.push(0);
    buf.push(0); // end of attribute list

    buf.push(0); // end of abbreviation table

    buf
}

// ─── .debug_info ───────────────────────────────────────────────────────────

fn build_debug_info(
    entries: &[FuncDebugEntry],
    file_indices: &HashMap<String, u32>,
    cu_low_pc: u32,
    cu_high_pc: u32,
    comp_dir: &str,
    comp_name: &str,
) -> Vec<u8> {
    let mut buf = Vec::new();

    // Reserve 4 bytes for unit_length (filled at end)
    buf.extend_from_slice(&[0u8; 4]);

    // version: 4
    buf.extend_from_slice(&4u16.to_le_bytes());
    // debug_abbrev_offset: 0
    buf.extend_from_slice(&0u32.to_le_bytes());
    // address_size: 4 (wasm32)
    buf.push(4);

    // Compile unit DIE (abbrev 1)
    write_uleb128(&mut buf, 1);
    write_string(&mut buf, comp_name); // DW_AT_name
    write_string(&mut buf, comp_dir); // DW_AT_comp_dir
    buf.push(DW_LANG_LO_USER as u8); // DW_AT_language (custom)
    buf.extend_from_slice(&cu_low_pc.to_le_bytes()); // DW_AT_low_pc
    buf.extend_from_slice(&(cu_high_pc - cu_low_pc).to_le_bytes()); // DW_AT_high_pc (offset)
    buf.extend_from_slice(&0u32.to_le_bytes()); // DW_AT_stmt_list (offset into .debug_line)

    // Subprogram DIEs (abbrev 2)
    for entry in entries {
        let (file_idx, line) = match (&entry.source_file, entry.source_line) {
            (Some(f), Some(l)) => {
                let idx = file_indices.get(f).copied().unwrap_or(0);
                (idx, l)
            }
            _ => continue, // skip functions without source info
        };

        write_uleb128(&mut buf, 2); // abbrev 2
        write_string(&mut buf, &entry.name); // DW_AT_name
        buf.extend_from_slice(&entry.code_offset.to_le_bytes()); // DW_AT_low_pc
        buf.extend_from_slice(&entry.code_size.to_le_bytes()); // DW_AT_high_pc (size)
        write_uleb128(&mut buf, file_idx as u64); // DW_AT_decl_file
        write_uleb128(&mut buf, line as u64); // DW_AT_decl_line
    }

    buf.push(0); // end of children (compile unit)

    // Patch unit_length (total size minus the 4-byte length field itself)
    let unit_length = (buf.len() - 4) as u32;
    buf[0..4].copy_from_slice(&unit_length.to_le_bytes());

    buf
}

// ─── .debug_line ───────────────────────────────────────────────────────────

fn build_debug_line(
    entries: &[FuncDebugEntry],
    file_indices: &HashMap<String, u32>,
    files: &[String],
) -> Vec<u8> {
    let mut buf = Vec::new();

    // Reserve 4 bytes for unit_length
    buf.extend_from_slice(&[0u8; 4]);

    // version: 4
    buf.extend_from_slice(&4u16.to_le_bytes());

    // Reserve 4 bytes for header_length
    let header_length_offset = buf.len();
    buf.extend_from_slice(&[0u8; 4]);

    let header_start = buf.len();

    // minimum_instruction_length: 1
    buf.push(1);
    // maximum_operations_per_instruction: 1 (DWARF v4)
    buf.push(1);
    // default_is_stmt: 1
    buf.push(1);
    // line_base: -5
    buf.push((-5i8) as u8);
    // line_range: 14
    buf.push(14);
    // opcode_base: 13 (standard opcodes 1-12)
    buf.push(13);
    // standard_opcode_lengths (opcodes 1-12)
    buf.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);

    // include_directories: empty (terminated by single 0)
    buf.push(0);

    // file_names
    for file in files {
        write_string(&mut buf, file); // file name
        write_uleb128(&mut buf, 0); // directory index (0 = comp_dir)
        write_uleb128(&mut buf, 0); // last modification time
        write_uleb128(&mut buf, 0); // file size
    }
    buf.push(0); // end of file names

    // Patch header_length
    let header_length = (buf.len() - header_start) as u32;
    buf[header_length_offset..header_length_offset + 4]
        .copy_from_slice(&header_length.to_le_bytes());

    // Line number program
    let mut prev_line: i64 = 1;

    for entry in entries {
        let (file_idx, line) = match (&entry.source_file, entry.source_line) {
            (Some(f), Some(l)) => {
                let idx = file_indices.get(f).copied().unwrap_or(0);
                (idx, l)
            }
            _ => continue,
        };

        // DW_LNE_set_address
        buf.push(0); // extended opcode marker
        write_uleb128(&mut buf, 5); // length: 1 (opcode) + 4 (addr)
        buf.push(DW_LNE_SET_ADDRESS);
        buf.extend_from_slice(&entry.code_offset.to_le_bytes());

        // DW_LNS_set_file
        buf.push(DW_LNS_SET_FILE);
        write_uleb128(&mut buf, file_idx as u64);

        // DW_LNS_advance_line
        let line_delta = line as i64 - prev_line;
        if line_delta != 0 {
            buf.push(DW_LNS_ADVANCE_LINE);
            write_sleb128(&mut buf, line_delta);
        }
        prev_line = line as i64;

        // DW_LNS_copy — emit a row in the line table
        buf.push(DW_LNS_COPY);
    }

    // End sequence
    // Advance PC past the last function
    if let Some(last) = entries.last() {
        buf.push(DW_LNS_ADVANCE_PC);
        write_uleb128(&mut buf, last.code_size as u64);
    }
    buf.push(0); // extended opcode marker
    write_uleb128(&mut buf, 1); // length: 1
    buf.push(DW_LNE_END_SEQUENCE);

    // Patch unit_length
    let unit_length = (buf.len() - 4) as u32;
    buf[0..4].copy_from_slice(&unit_length.to_le_bytes());

    buf
}

// ─── Public API ────────────────────────────────────────────────────────────

/// Append DWARF v4 debug sections (.debug_abbrev, .debug_info, .debug_line)
/// as WASM custom sections.
pub(super) fn append_dwarf_sections(wasm: &mut Vec<u8>, entries: &[FuncDebugEntry]) {
    if entries.is_empty() {
        return;
    }

    // Only include entries that have source info
    let entries_with_source: Vec<&FuncDebugEntry> = entries
        .iter()
        .filter(|e| e.source_file.is_some() && e.source_line.is_some())
        .collect();
    if entries_with_source.is_empty() {
        return;
    }

    // Build file table (1-based indices)
    let mut files: Vec<String> = Vec::new();
    let mut file_indices: HashMap<String, u32> = HashMap::new();
    for entry in &entries_with_source {
        if let Some(f) = &entry.source_file {
            if !file_indices.contains_key(f) {
                files.push(f.clone());
                file_indices.insert(f.clone(), files.len() as u32); // 1-based
            }
        }
    }

    // Compute CU address range
    let cu_low_pc = entries.first().map(|e| e.code_offset).unwrap_or(0);
    let cu_high_pc = entries
        .last()
        .map(|e| e.code_offset + e.code_size)
        .unwrap_or(0);

    // Determine compilation unit name/dir from first source file
    let comp_name = files.first().map(|f| f.as_str()).unwrap_or("<unknown>");
    let comp_dir = ".";

    // Build sections
    let abbrev = build_debug_abbrev();
    let info = build_debug_info(
        entries,
        &file_indices,
        cu_low_pc,
        cu_high_pc,
        comp_dir,
        comp_name,
    );
    let line = build_debug_line(entries, &file_indices, &files);

    // Append as WASM custom sections
    for (name, data) in [
        (".debug_abbrev", &abbrev),
        (".debug_info", &info),
        (".debug_line", &line),
    ] {
        let section = CustomSection {
            name: Cow::Borrowed(name),
            data: Cow::Borrowed(data),
        };
        let mut tmp = Module::new();
        tmp.section(&section);
        let encoded = tmp.finish();
        // Skip the 8-byte WASM module header (magic + version)
        wasm.extend_from_slice(&encoded[8..]);
    }
}
