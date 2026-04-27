use nexus_wasm_alloc::{checked_ptr_len, read_string};

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_mark() -> i32 {
    nexus_wasm_alloc::mark()
}

#[cfg(not(feature = "no_alloc_export"))]
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_alloc_reset(mark: i32) {
    nexus_wasm_alloc::reset_to(mark);
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_length(s_ptr: i32, s_len: i32) -> i64 {
    read_string(s_ptr, s_len).chars().count() as i64
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_byte_length(_s_ptr: i32, s_len: i32) -> i64 {
    s_len as i64
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_contains(s_ptr: i32, s_len: i32, sub_ptr: i32, sub_len: i32) -> i32 {
    let s = read_string(s_ptr, s_len);
    let sub = read_string(sub_ptr, sub_len);
    if s.contains(sub.as_str()) {
        1
    } else {
        0
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_substring(s_ptr: i32, s_len: i32, start: i64, len: i64) -> i64 {
    let s = read_string(s_ptr, s_len);
    let start = start.max(0) as usize;
    let len = len.max(0) as usize;
    let result: String = s.chars().skip(start).take(len).collect();
    nexus_wasm_alloc::store_string_result(result)
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_index_of(s_ptr: i32, s_len: i32, sub_ptr: i32, sub_len: i32) -> i64 {
    let s = read_string(s_ptr, s_len);
    let sub = read_string(sub_ptr, sub_len);
    s.find(sub.as_str()).map(|i| i as i64).unwrap_or(-1)
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_starts_with(
    s_ptr: i32,
    s_len: i32,
    prefix_ptr: i32,
    prefix_len: i32,
) -> i32 {
    let s = read_string(s_ptr, s_len);
    let prefix = read_string(prefix_ptr, prefix_len);
    if s.starts_with(prefix.as_str()) {
        1
    } else {
        0
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_ends_with(
    s_ptr: i32,
    s_len: i32,
    suffix_ptr: i32,
    suffix_len: i32,
) -> i32 {
    let s = read_string(s_ptr, s_len);
    let suffix = read_string(suffix_ptr, suffix_len);
    if s.ends_with(suffix.as_str()) {
        1
    } else {
        0
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_trim(s_ptr: i32, s_len: i32) -> i64 {
    nexus_wasm_alloc::store_string_result(read_string(s_ptr, s_len).trim().to_string())
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_to_upper(s_ptr: i32, s_len: i32) -> i64 {
    nexus_wasm_alloc::store_string_result(read_string(s_ptr, s_len).to_uppercase())
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_to_lower(s_ptr: i32, s_len: i32) -> i64 {
    nexus_wasm_alloc::store_string_result(read_string(s_ptr, s_len).to_lowercase())
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_replace(
    s_ptr: i32,
    s_len: i32,
    from_ptr: i32,
    from_len: i32,
    to_ptr: i32,
    to_len: i32,
) -> i64 {
    let s = read_string(s_ptr, s_len);
    let from = read_string(from_ptr, from_len);
    let to = read_string(to_ptr, to_len);
    nexus_wasm_alloc::store_string_result(s.replace(from.as_str(), to.as_str()))
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_char_at(s_ptr: i32, s_len: i32, idx: i64) -> i32 {
    if idx < 0 {
        return 0;
    }
    let s = read_string(s_ptr, s_len);
    match s.chars().nth(idx as usize) {
        Some(c) => c as i32,
        None => 0,
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_from_char(c: i32) -> i64 {
    match char::from_u32(c as u32) {
        Some(ch) => nexus_wasm_alloc::store_string_result(ch.to_string()),
        None => 0,
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_from_i64(val: i64) -> i64 {
    nexus_wasm_alloc::store_string_result(val.to_string())
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_from_float(val: f64) -> i64 {
    nexus_wasm_alloc::store_string_result(val.to_string())
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_from_bool(val: i32) -> i64 {
    let b = val != 0;
    nexus_wasm_alloc::store_string_result(b.to_string())
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_to_i64(s_ptr: i32, s_len: i32) -> i64 {
    let s = read_string(s_ptr, s_len);
    match s.trim().parse::<i64>() {
        Ok(n) => n,
        Err(_) => {
            panic!("to_i64: invalid integer '{}'", s);
        }
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_repeat(s_ptr: i32, s_len: i32, n: i64) -> i64 {
    let s = read_string(s_ptr, s_len);
    let count = n.max(0) as usize;
    nexus_wasm_alloc::store_string_result(s.repeat(count))
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_pad_left(
    s_ptr: i32,
    s_len: i32,
    width: i64,
    fill_ptr: i32,
    fill_len: i32,
) -> i64 {
    let s = read_string(s_ptr, s_len);
    let fill = read_string(fill_ptr, fill_len);
    let w = width.max(0) as usize;
    let char_count = s.chars().count();
    if char_count >= w || fill.is_empty() {
        nexus_wasm_alloc::store_string_result(s)
    } else {
        let pad_chars = w - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        let padding: String = fill_chars.iter().cycle().take(pad_chars).collect();
        nexus_wasm_alloc::store_string_result(format!("{}{}", padding, s))
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_pad_right(
    s_ptr: i32,
    s_len: i32,
    width: i64,
    fill_ptr: i32,
    fill_len: i32,
) -> i64 {
    let s = read_string(s_ptr, s_len);
    let fill = read_string(fill_ptr, fill_len);
    let w = width.max(0) as usize;
    let char_count = s.chars().count();
    if char_count >= w || fill.is_empty() {
        nexus_wasm_alloc::store_string_result(s)
    } else {
        let pad_chars = w - char_count;
        let fill_chars: Vec<char> = fill.chars().collect();
        let padding: String = fill_chars.iter().cycle().take(pad_chars).collect();
        nexus_wasm_alloc::store_string_result(format!("{}{}", s, padding))
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_is_valid_i64(s_ptr: i32, s_len: i32) -> i32 {
    let s = read_string(s_ptr, s_len);
    if s.trim().parse::<i64>().is_ok() { 1 } else { 0 }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_char_code(s_ptr: i32, s_len: i32, idx: i64) -> i64 {
    if idx < 0 {
        return -1;
    }
    let s = read_string(s_ptr, s_len);
    match s.chars().nth(idx as usize) {
        Some(c) => c as i64,
        None => -1,
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_char_ord(c: i32) -> i64 {
    c as i64
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_from_char_code(code: i64) -> i64 {
    match char::from_u32(code as u32) {
        Some(c) => nexus_wasm_alloc::store_string_result(c.to_string()),
        None => 0,
    }
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_to_f64(s_ptr: i32, s_len: i32) -> f64 {
    let s = read_string(s_ptr, s_len);
    s.trim().parse::<f64>().unwrap_or(f64::NAN)
}

#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_is_valid_f64(s_ptr: i32, s_len: i32) -> i32 {
    let s = read_string(s_ptr, s_len);
    match s.trim().parse::<f64>() {
        Ok(v) if !v.is_nan() || s.trim() == "NaN" => 1,
        _ => 0,
    }
}

// ─── Byte-level scanning primitives for lexer performance ────────────────────

fn raw_bytes(s_ptr: i32, s_len: i32) -> Option<&'static [u8]> {
    let (offset, len) = checked_ptr_len(s_ptr, s_len)?;
    Some(unsafe { std::slice::from_raw_parts(offset as *const u8, len) })
}

/// Returns the raw byte value at the given byte position (0-255). O(1).
/// Returns 0 if out of bounds.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_byte_at(s_ptr: i32, s_len: i32, idx: i64) -> i32 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return 0;
    };
    if idx < 0 || (idx as usize) >= bytes.len() {
        return 0;
    }
    bytes[idx as usize] as i32
}

/// Scans from `start` while characters are ASCII identifier chars (a-z, A-Z, 0-9, _).
/// Returns the byte position of the first non-ident character.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_scan_ident(s_ptr: i32, s_len: i32, start: i64) -> i64 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return start;
    };
    let mut i = (start.max(0) as usize).min(bytes.len());
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_alphanumeric() || b == b'_' {
            i += 1;
        } else {
            break;
        }
    }
    i as i64
}

/// Scans from `start` while characters are ASCII digits (0-9).
/// Returns the byte position of the first non-digit character.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_scan_digits(s_ptr: i32, s_len: i32, start: i64) -> i64 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return start;
    };
    let mut i = (start.max(0) as usize).min(bytes.len());
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    i as i64
}

/// Skips ASCII whitespace (space, tab, \n, \r) from `start`.
/// Returns the byte position of the first non-whitespace character.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_skip_ws(s_ptr: i32, s_len: i32, start: i64) -> i64 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return start;
    };
    let mut i = (start.max(0) as usize).min(bytes.len());
    while i < bytes.len() {
        let b = bytes[i];
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
        } else {
            break;
        }
    }
    i as i64
}

/// Counts the number of newline characters (\n) in the byte range [start, end).
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_count_newlines_in(s_ptr: i32, s_len: i32, start: i64, end: i64) -> i64 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return 0;
    };
    let start = (start.max(0) as usize).min(bytes.len());
    let end = (end.max(0) as usize).min(bytes.len());
    bytes[start..end].iter().filter(|&&b| b == b'\n').count() as i64
}

/// Returns the byte position of the last newline (\n) in range [start, end),
/// or -1 if no newline is found.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_last_newline_in(s_ptr: i32, s_len: i32, start: i64, end: i64) -> i64 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return -1;
    };
    let start = (start.max(0) as usize).min(bytes.len());
    let end = (end.max(0) as usize).min(bytes.len());
    for i in (start..end).rev() {
        if bytes[i] == b'\n' {
            return i as i64;
        }
    }
    -1
}

/// Finds the first occurrence of byte `ch` starting from `start`.
/// Returns the byte position, or -1 if not found.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_find_byte(s_ptr: i32, s_len: i32, start: i64, ch: i32) -> i64 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return -1;
    };
    let start = (start.max(0) as usize).min(bytes.len());
    let target = ch as u8;
    for i in start..bytes.len() {
        if bytes[i] == target {
            return i as i64;
        }
    }
    -1
}

/// Extracts a substring by byte offset and byte length. O(len).
/// Unlike `substring` which uses character indices, this uses raw byte positions.
#[cfg_attr(not(feature = "component"), no_mangle)]
pub extern "C" fn __nx_string_byte_substring(s_ptr: i32, s_len: i32, start: i64, len: i64) -> i64 {
    let Some(bytes) = raw_bytes(s_ptr, s_len) else {
        return 0;
    };
    let start = (start.max(0) as usize).min(bytes.len());
    let len = (len.max(0) as usize).min(bytes.len().saturating_sub(start));
    if len == 0 {
        return 0;
    }
    let result = String::from_utf8_lossy(&bytes[start..start + len]).to_string();
    nexus_wasm_alloc::store_string_result(result)
}
