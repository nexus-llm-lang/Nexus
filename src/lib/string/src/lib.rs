use nexus_wasm_alloc::read_string;

#[cfg(not(feature = "no_alloc_export"))]
#[no_mangle]
pub extern "C" fn allocate(size: i32) -> i32 {
    nexus_wasm_alloc::allocate(size)
}

#[cfg(not(feature = "no_alloc_export"))]
#[no_mangle]
pub unsafe extern "C" fn deallocate(ptr: i32, size: i32) {
    nexus_wasm_alloc::deallocate(ptr, size);
}

#[no_mangle]
pub extern "C" fn __nx_string_length(s_ptr: i32, s_len: i32) -> i64 {
    read_string(s_ptr, s_len).len() as i64
}

#[no_mangle]
pub extern "C" fn __nx_string_contains(s_ptr: i32, s_len: i32, sub_ptr: i32, sub_len: i32) -> i32 {
    let s = read_string(s_ptr, s_len);
    let sub = read_string(sub_ptr, sub_len);
    if s.contains(sub.as_str()) {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn __nx_string_substring(s_ptr: i32, s_len: i32, start: i64, len: i64) -> i64 {
    let s = read_string(s_ptr, s_len);
    let start = start.max(0) as usize;
    let len = len.max(0) as usize;
    let result: String = s.chars().skip(start).take(len).collect();
    nexus_wasm_alloc::store_string_result(result)
}

#[no_mangle]
pub extern "C" fn __nx_string_index_of(s_ptr: i32, s_len: i32, sub_ptr: i32, sub_len: i32) -> i64 {
    let s = read_string(s_ptr, s_len);
    let sub = read_string(sub_ptr, sub_len);
    s.find(sub.as_str()).map(|i| i as i64).unwrap_or(-1)
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_string_trim(s_ptr: i32, s_len: i32) -> i64 {
    nexus_wasm_alloc::store_string_result(read_string(s_ptr, s_len).trim().to_string())
}

#[no_mangle]
pub extern "C" fn __nx_string_to_upper(s_ptr: i32, s_len: i32) -> i64 {
    nexus_wasm_alloc::store_string_result(read_string(s_ptr, s_len).to_uppercase())
}

#[no_mangle]
pub extern "C" fn __nx_string_to_lower(s_ptr: i32, s_len: i32) -> i64 {
    nexus_wasm_alloc::store_string_result(read_string(s_ptr, s_len).to_lowercase())
}

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_string_char_at(s_ptr: i32, s_len: i32, idx: i64) -> i64 {
    if idx < 0 {
        return 0;
    }
    let s = read_string(s_ptr, s_len);
    match s.chars().nth(idx as usize) {
        Some(c) => nexus_wasm_alloc::store_string_result(c.to_string()),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn __nx_string_from_i64(val: i64) -> i64 {
    nexus_wasm_alloc::store_string_result(val.to_string())
}

#[no_mangle]
pub extern "C" fn __nx_string_from_float(val: f64) -> i64 {
    nexus_wasm_alloc::store_string_result(val.to_string())
}

#[no_mangle]
pub extern "C" fn __nx_string_from_bool(val: i32) -> i64 {
    let b = val != 0;
    nexus_wasm_alloc::store_string_result(b.to_string())
}

#[no_mangle]
pub extern "C" fn __nx_string_to_i64(s_ptr: i32, s_len: i32) -> i64 {
    let s = read_string(s_ptr, s_len);
    match s.trim().parse::<i64>() {
        Ok(n) => n,
        Err(_) => {
            panic!("to_i64: invalid integer '{}'", s);
        }
    }
}

#[no_mangle]
pub extern "C" fn __nx_string_repeat(s_ptr: i32, s_len: i32, n: i64) -> i64 {
    let s = read_string(s_ptr, s_len);
    let count = n.max(0) as usize;
    nexus_wasm_alloc::store_string_result(s.repeat(count))
}

#[no_mangle]
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

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn __nx_string_is_valid_i64(s_ptr: i32, s_len: i32) -> i32 {
    let s = read_string(s_ptr, s_len);
    if s.trim().parse::<i64>().is_ok() { 1 } else { 0 }
}
